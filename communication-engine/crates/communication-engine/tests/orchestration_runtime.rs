//! Phase 7 end to end through the public facade: fake audio → scripted ASR
//! → semantic rules → scripted translation, coordinated as a pipeline.
//! Graph validation before activation, epochs and generations, degraded
//! and failed operation, restart recovery, reset, stop modes and outcomes,
//! hung workers, multi-stream isolation, deadlines, privacy and bounded
//! repeated lifecycles. Deterministic inputs; no network, no model. The
//! engine clock is a manual test clock, so latency values here are not
//! real-time measurements (see `phase-7-evaluation.md`).

mod common;

use std::time::Duration;

use ce_testkit::fake_audio::FakeDriver;
use common::*;
use communication_engine::contracts::config::EngineConfig;
use communication_engine::contracts::orchestration::{
    DeadlinePolicy, FaultClass, PipelineEndReason, StageState, StopOutcome,
};
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

fn echo(id: &str) -> ScriptedFactory {
    ScriptedFactory::echo(scripted_capabilities(id, &[("en", "es")]))
}

fn recognition() -> StageSpec {
    StageSpec::required(StageKind::Recognition)
        .with_config(StageConfig::Recognition(Box::new(continuous())))
}

fn translation(requirement: Requirement, adapter: &str) -> StageSpec {
    StageSpec {
        kind: StageKind::Translation,
        requirement,
        config: Some(StageConfig::Translation(Box::new(en_es(adapter)))),
    }
}

fn full(adapter: &str) -> PipelineSpec {
    PipelineSpec::new(vec![
        translation(Requirement::Required, adapter),
        StageSpec::optional(StageKind::Semantics),
        recognition(),
    ])
}

fn utterances() -> ScriptedAsr {
    ScriptedAsr::new(vec![
        UtteranceScript::progressive("the total is forty dollars", 80).endpoint_at(500),
        UtteranceScript::progressive("can you split it", 80).endpoint_at(400),
    ])
}

fn many_utterances(n: usize) -> ScriptedAsr {
    ScriptedAsr::new(
        (0..n)
            .map(|_| UtteranceScript::progressive("one more thing", 60).endpoint_at(300))
            .collect(),
    )
}

async fn harness_with(
    config: EngineConfig,
    asr: ScriptedAsr,
    adapters: Vec<ScriptedFactory>,
) -> AudioHarness {
    audio_harness_custom(config, FakeDriver::Manual, move |mut b| {
        b = b.asr_adapter(asr);
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

async fn feed(h: &AudioHarness, readers: &mut [&mut CaptureReader], blocks: usize) {
    for _ in 0..blocks {
        h.fake.tick(160);
        for r in readers.iter_mut() {
            let _ = recv_block(r).await;
        }
    }
}

fn orch(h: &AudioHarness) -> Vec<(OrchestrationEvent, DataClass)> {
    h.events
        .events()
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::Orchestration(o) => Some((o.clone(), e.privacy)),
            _ => None,
        })
        .collect()
}

async fn wait_orch(
    h: &mut AudioHarness,
    pred: impl Fn(&OrchestrationEvent) -> bool,
) -> OrchestrationEvent {
    if let Some((e, _)) = orch(h).into_iter().find(|(e, _)| pred(e)) {
        return e;
    }
    let e = h
        .events
        .wait_for(
            WAIT,
            |e| matches!(&e.payload, EventPayload::Orchestration(o) if pred(o)),
        )
        .await;
    let Some(e) = e else {
        let kinds: Vec<&str> = orch(h).iter().map(|(e, _)| e.kind()).collect();
        panic!("expected orchestration event; got {kinds:?}");
    };
    match &e.payload {
        EventPayload::Orchestration(o) => o.clone(),
        _ => unreachable!(),
    }
}

async fn wait_final_translation(h: &mut AudioHarness, stream: AudioStreamId, utterance: u64) {
    let pred = |e: &EventEnvelope| {
        matches!(&e.payload, EventPayload::Translation(TranslationEvent::Result(r))
            if r.source.stream == stream && r.source.utterance == utterance
            && r.state == TranslationState::Final)
    };
    if h.events.events().iter().any(|e| pred(e)) {
        return;
    }
    let found = h.events.wait_for(WAIT, pred).await;
    assert!(
        found.is_some(),
        "no final translation of utterance {utterance}"
    );
}

fn code<T: std::fmt::Debug>(r: Result<T, CeError>) -> String {
    r.unwrap_err().code().to_owned()
}

#[tokio::test]
async fn a_pipeline_runs_end_to_end_with_epochs_generations_and_no_findings() {
    let mut h = harness_with(audio_config(), utterances(), vec![echo("t.echo")]).await;
    h.grant_capture().await;
    let (mut reader, id) = open(&h).await;
    let st = h.engine.start_pipeline(id, full("t.echo")).await.unwrap();
    assert_eq!(st.state, PipelineState::Running);
    let order: Vec<StageKind> = st.stages.iter().map(|s| s.kind).collect();
    assert_eq!(
        order,
        vec![
            StageKind::Recognition,
            StageKind::Translation,
            StageKind::Semantics
        ],
        "dependency order, not declaration order"
    );
    let gens: Vec<u64> = st.stages.iter().map(|s| s.generation.unwrap()).collect();
    assert!(gens.windows(2).all(|w| w[0] != w[1]), "{gens:?}");
    assert!(st.notes.is_empty(), "{:?}", st.notes);
    feed(&h, &mut [&mut reader], 120).await;
    wait_final_translation(&mut h, id, 1).await;

    let st = h.engine.pipeline_status(id).await.unwrap();
    let c = &st.counters;
    assert_eq!(
        (
            c.causal_violations,
            c.timeline_anomalies,
            c.stale_outputs_rejected
        ),
        (0, 0, 0),
        "real Phase 4–6 producers satisfy the orchestration checks: {c:?}"
    );
    assert!(
        c.transcript_revisions >= 4 && c.derived_outputs >= 2,
        "{c:?}"
    );
    assert_eq!(c.translation_runs, 1);
    assert_eq!(st.timeline.sample_rate, 16_000);
    assert_eq!(st.timeline.segments, 1);
    let metrics: Vec<_> = st.latency.iter().map(|l| l.metric).collect();
    for m in [
        communication_engine::contracts::orchestration::LatencyMetric::TranscriptFinal,
        communication_engine::contracts::orchestration::LatencyMetric::TranslationFinal,
        communication_engine::contracts::orchestration::LatencyMetric::Overhead,
    ] {
        assert!(metrics.contains(&m), "{metrics:?}");
    }
    // Every orchestration event is operational and carries no content.
    for (e, class) in orch(&h) {
        assert_eq!(class, DataClass::Operational);
        let j = serde_json::to_string(&e).unwrap().to_lowercase();
        assert!(!j.contains("forty") && !j.contains("split"), "{j}");
    }
    let started = orch(&h)
        .iter()
        .filter(|(e, _)| matches!(e, OrchestrationEvent::StageStarted { .. }))
        .count();
    assert_eq!(started, 3);
    // Drain stop: recognition first; downstream workers finish and end.
    let st = h.engine.stop_pipeline(id).await.unwrap();
    assert_eq!(st.state, PipelineState::Stopped);
    assert_eq!(st.ended, Some(PipelineEndReason::Stopped));
    match wait_orch(&mut h, |e| {
        matches!(e, OrchestrationEvent::PipelineStopped { .. })
    })
    .await
    {
        OrchestrationEvent::PipelineStopped { outcomes, .. } => {
            assert_eq!(outcomes[0], (StageKind::Recognition, StopOutcome::Clean));
            assert!(
                outcomes.iter().all(|(_, o)| *o == StopOutcome::Clean),
                "{outcomes:?}"
            );
        }
        _ => unreachable!(),
    }
    let m = h.engine.metrics();
    assert_eq!((m.pipelines_started, m.pipelines_failed), (1, 0));
    assert_eq!(h.engine.health().await.unwrap().pipelines.len(), 1);
    assert!(h.engine.shutdown().await.unwrap().clean);
}

#[tokio::test]
async fn invalid_pipelines_fail_before_any_stage_starts() {
    let mut config = audio_config();
    config.orchestration.max_pipelines = 1;
    let h = harness_with(config, utterances(), vec![echo("t.echo")]).await;
    h.grant_capture().await;
    let (_r1, a) = open(&h).await;
    let (_r2, b) = open(&h).await;
    // Missing dependency.
    let only_tr = PipelineSpec::new(vec![translation(Requirement::Required, "t.echo")]);
    assert_eq!(
        code(h.engine.start_pipeline(a, only_tr).await),
        "orchestration.missing_dependency"
    );
    // The engine default recognition segments with activity hints: it needs analysis.
    let default_rec = PipelineSpec::new(vec![StageSpec::required(StageKind::Recognition)]);
    assert_eq!(
        code(h.engine.start_pipeline(a, default_rec).await),
        "orchestration.missing_dependency"
    );
    // A required stage that depends on an optional one.
    let mut weak = full("t.echo");
    weak.stages[2].requirement = Requirement::Optional;
    assert_eq!(
        code(h.engine.start_pipeline(a, weak).await),
        "orchestration.required_depends_on_optional"
    );
    // Pre-flight: a required adapter that is not registered.
    assert_eq!(
        code(h.engine.start_pipeline(a, full("missing")).await),
        "translation.no_adapter"
    );
    // Nothing was started by the failed attempts.
    assert!(h.engine.recognition_status(a).await.is_err());
    assert!(h.engine.translation_status(a).await.is_err());
    let unknown = AudioStreamId::from_uuid(uuid::Uuid::from_u128(5));
    assert_eq!(
        code(h.engine.start_pipeline(unknown, full("t.echo")).await),
        "audio.unknown_stream"
    );
    h.engine.start_pipeline(a, full("t.echo")).await.unwrap();
    assert_eq!(
        code(h.engine.start_pipeline(a, full("t.echo")).await),
        "orchestration.already_running"
    );
    assert_eq!(
        code(h.engine.start_pipeline(b, full("t.echo")).await),
        "orchestration.capacity_exhausted"
    );
    // A stage already running outside a pipeline is not taken over.
    h.engine
        .start_recognition(b, Some(continuous()))
        .await
        .unwrap();
    h.engine.stop_pipeline(a).await.unwrap();
    assert_eq!(
        code(h.engine.start_pipeline(b, full("t.echo")).await),
        "orchestration.stage_already_running"
    );
    assert_eq!(
        code(h.engine.pipeline_status(unknown).await),
        "orchestration.not_found"
    );
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn optional_stage_failures_degrade_explicitly_and_the_rest_keeps_running() {
    let bad = echo("t.bad").with_faults(Faults {
        panic_at: Some(0),
        ..Faults::default()
    });
    let mut h = harness_with(audio_config(), utterances(), vec![echo("t.echo"), bad]).await;
    h.grant_capture().await;
    let (mut reader, id) = open(&h).await;
    // Start-time: an optional stage that cannot start.
    let spec = PipelineSpec::new(vec![
        recognition(),
        translation(Requirement::Optional, "missing"),
    ]);
    let st = h.engine.start_pipeline(id, spec).await.unwrap();
    assert_eq!(st.state, PipelineState::Degraded);
    let tr = st
        .stages
        .iter()
        .find(|s| s.kind == StageKind::Translation)
        .unwrap();
    assert_eq!(tr.state, StageState::Degraded);
    assert_eq!(tr.reason.as_deref(), Some("translation.no_adapter"));
    match wait_orch(&mut h, |e| {
        matches!(e, OrchestrationEvent::StageDegraded { .. })
    })
    .await
    {
        OrchestrationEvent::StageDegraded {
            stage, fault, code, ..
        } => {
            assert_eq!(stage, StageKind::Translation);
            assert_eq!(fault, FaultClass::UnavailableCapability);
            assert_eq!(code, "translation.no_adapter");
        }
        _ => unreachable!(),
    }
    h.engine.stop_pipeline(id).await.unwrap();

    // Run-time: an optional stage whose worker panics.
    let spec = PipelineSpec::new(vec![
        recognition(),
        translation(Requirement::Optional, "t.bad"),
    ]);
    h.engine.start_pipeline(id, spec).await.unwrap();
    feed(&h, &mut [&mut reader], 70).await;
    match wait_orch(&mut h, |e| {
        matches!(
            e,
            OrchestrationEvent::StageDegraded {
                fault: FaultClass::WorkerFailure,
                ..
            }
        )
    })
    .await
    {
        OrchestrationEvent::StageDegraded { stage, fault, .. } => {
            assert_eq!(
                (stage, fault),
                (StageKind::Translation, FaultClass::WorkerFailure)
            );
        }
        _ => unreachable!(),
    }
    let st = h.engine.pipeline_status(id).await.unwrap();
    assert_eq!(st.state, PipelineState::Degraded);
    // Recognition is unaffected: the second utterance still finalizes.
    feed(&h, &mut [&mut reader], 60).await;
    let fin = h
        .events
        .wait_for(WAIT, |e| {
            matches!(&e.payload, EventPayload::Speech(SpeechEvent::Transcript(r))
                if r.meta.utterance == 1 && r.status == RevisionStatus::Final)
        })
        .await;
    assert!(fin.is_some());
    assert!(h.engine.recognition_status(id).await.unwrap().running);
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_required_failure_stops_the_pipeline_and_restart_recovers_with_a_new_epoch() {
    let bad = echo("t.bad").with_faults(Faults {
        panic_at: Some(0),
        ..Faults::default()
    });
    let mut h = harness_with(audio_config(), utterances(), vec![bad, echo("t.echo")]).await;
    h.grant_capture().await;
    let (mut reader, id) = open(&h).await;
    let first = h
        .engine
        .start_pipeline(
            id,
            PipelineSpec::new(vec![
                recognition(),
                translation(Requirement::Required, "t.bad"),
            ]),
        )
        .await
        .unwrap();
    feed(&h, &mut [&mut reader], 70).await;
    match wait_orch(&mut h, |e| {
        matches!(e, OrchestrationEvent::PipelineFailed { .. })
    })
    .await
    {
        OrchestrationEvent::PipelineFailed {
            stage,
            fault,
            error,
            ..
        } => {
            assert_eq!(stage, Some(StageKind::Translation));
            assert_eq!(fault, FaultClass::WorkerFailure);
            assert_eq!(error.code, "orchestration.required_stage_failed");
        }
        _ => unreachable!(),
    }
    match wait_orch(&mut h, |e| {
        matches!(e, OrchestrationEvent::PipelineStopped { .. })
    })
    .await
    {
        OrchestrationEvent::PipelineStopped { reason, .. } => {
            assert_eq!(reason, PipelineEndReason::Failed)
        }
        _ => unreachable!(),
    }
    let st = h.engine.pipeline_status(id).await.unwrap();
    assert_eq!(st.state, PipelineState::Failed);
    assert!(
        !h.engine.recognition_status(id).await.unwrap().running,
        "the remaining stages were stopped"
    );
    assert_eq!(h.engine.metrics().pipelines_failed, 1);
    // Recovery: restart with a working adapter; a new epoch and new generations.
    let second = h
        .engine
        .restart_pipeline(
            id,
            Some(PipelineSpec::new(vec![
                recognition(),
                translation(Requirement::Required, "t.echo"),
            ])),
        )
        .await
        .unwrap();
    assert!(second.epoch > first.epoch);
    let old: Vec<u64> = first.stages.iter().filter_map(|s| s.generation).collect();
    assert!(
        second
            .stages
            .iter()
            .all(|s| s.generation.is_some_and(|g| old.iter().all(|o| g > *o)))
    );
    feed(&h, &mut [&mut reader], 70).await;
    let found = h
        .events
        .wait_for(WAIT, |e| {
            matches!(&e.payload, EventPayload::Translation(TranslationEvent::Result(r))
                if r.state == TranslationState::Final)
        })
        .await;
    assert!(found.is_some(), "the recovered pipeline translates");
    let st = h.engine.pipeline_status(id).await.unwrap();
    assert_eq!(st.state, PipelineState::Running);
    assert_eq!(st.counters.causal_violations, 0, "{:?}", st.counters);
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn restart_and_reset_keep_downstream_state_consistent() {
    let mut h = harness_with(audio_config(), many_utterances(8), vec![echo("t.echo")]).await;
    h.grant_capture().await;
    let (mut reader, id) = open(&h).await;
    let a = h.engine.start_pipeline(id, full("t.echo")).await.unwrap();
    feed(&h, &mut [&mut reader], 45).await;
    // Reset mid-stream: identifiers continue, reset epoch advances.
    let st = h.engine.reset_pipeline(id).await.unwrap();
    assert_eq!((st.epoch, st.reset_epoch), (a.epoch, 1));
    wait_orch(&mut h, |e| {
        matches!(e, OrchestrationEvent::PipelineReset { reset_epoch: 1, .. })
    })
    .await;
    feed(&h, &mut [&mut reader], 45).await;
    let st = h.engine.pipeline_status(id).await.unwrap();
    assert_eq!(st.counters.causal_violations, 0, "{:?}", st.counters);
    // Restart: the translation worker restarts (Started), sequences restart
    // at 0, and the new observer sees no violation.
    let b = h.engine.restart_pipeline(id, None).await.unwrap();
    assert!(b.epoch > a.epoch);
    assert_eq!(b.reset_epoch, 0);
    assert_eq!(b.fingerprint, a.fingerprint, "same specification");
    // Everything published from here on belongs to the new epoch (outputs of
    // the previous workers are rejected, never published).
    let new_epoch = b.epoch;
    wait_orch(&mut h, |e| {
        matches!(e, OrchestrationEvent::StageStarted { epoch, stage: StageKind::Translation, .. }
            if *epoch == new_epoch)
    })
    .await;
    feed(&h, &mut [&mut reader], 70).await;
    let found = h
        .events
        .wait_for(WAIT, |e| {
            matches!(&e.payload, EventPayload::Translation(TranslationEvent::Result(r))
                if r.source.utterance == 0 && r.sequence == 0)
        })
        .await;
    assert!(found.is_some(), "the new worker run numbers from 0 again");
    wait_final_translation(&mut h, id, 0).await;
    let st = h.engine.pipeline_status(id).await.unwrap();
    assert_eq!(
        (st.counters.causal_violations, st.counters.translation_runs),
        (0, 1),
        "{:?}",
        st.counters
    );
    let stopped: Vec<PipelineEndReason> = orch(&h)
        .iter()
        .filter_map(|(e, _)| match e {
            OrchestrationEvent::PipelineStopped { reason, .. } => Some(*reason),
            _ => None,
        })
        .collect();
    assert_eq!(stopped, vec![PipelineEndReason::Restart]);
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancel_stop_reports_a_hung_stage_as_timed_out_and_stays_bounded() {
    let hung = echo("t.hung").with_faults(Faults {
        hang_at: Some((0, Duration::from_millis(2_000))),
        ..Faults::default()
    });
    let mut h = harness_with(audio_config(), utterances(), vec![hung]).await;
    h.grant_capture().await;
    let (mut reader, id) = open(&h).await;
    let mut spec = PipelineSpec::new(vec![
        recognition(),
        translation(Requirement::Required, "t.hung"),
    ]);
    spec.stop = StopMode::Cancel;
    h.engine.start_pipeline(id, spec).await.unwrap();
    feed(&h, &mut [&mut reader], 70).await;
    // Wait until the translation worker is inside the hung adapter call.
    let busy = h
        .events
        .wait_for(WAIT, |e| {
            matches!(&e.payload, EventPayload::Speech(SpeechEvent::Transcript(r))
                if r.status == RevisionStatus::Partial)
        })
        .await;
    assert!(busy.is_some());
    tokio::time::sleep(Duration::from_millis(50)).await;
    let st = h.engine.stop_pipeline(id).await.unwrap();
    let tr = st
        .stages
        .iter()
        .find(|s| s.kind == StageKind::Translation)
        .unwrap();
    assert_eq!(tr.stop, Some(StopOutcome::TimedOut), "{st:?}");
    let rec = st
        .stages
        .iter()
        .find(|s| s.kind == StageKind::Recognition)
        .unwrap();
    assert_eq!(rec.stop, Some(StopOutcome::Clean));
    // Cancel order: downstream first.
    match wait_orch(&mut h, |e| {
        matches!(e, OrchestrationEvent::PipelineStopped { .. })
    })
    .await
    {
        OrchestrationEvent::PipelineStopped { outcomes, .. } => {
            assert_eq!(outcomes[0].0, StageKind::Translation)
        }
        _ => unreachable!(),
    }
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn pipelines_on_two_streams_are_isolated() {
    let bad = echo("t.bad").with_faults(Faults {
        panic_at: Some(0),
        ..Faults::default()
    });
    let mut h = harness_with(audio_config(), utterances(), vec![echo("t.echo"), bad]).await;
    h.grant_capture().await;
    let (mut ra, a) = open(&h).await;
    let (mut rb, b) = open(&h).await;
    h.engine
        .start_pipeline(
            a,
            PipelineSpec::new(vec![
                recognition(),
                translation(Requirement::Required, "t.bad"),
            ]),
        )
        .await
        .unwrap();
    h.engine.start_pipeline(b, full("t.echo")).await.unwrap();
    feed(&h, &mut [&mut ra, &mut rb], 120).await;
    wait_orch(
        &mut h,
        |e| matches!(e, OrchestrationEvent::PipelineFailed { stream, .. } if *stream == a),
    )
    .await;
    wait_final_translation(&mut h, b, 1).await;
    let sb = h.engine.pipeline_status(b).await.unwrap();
    assert_eq!(sb.state, PipelineState::Running);
    assert_eq!(sb.counters.causal_violations, 0);
    assert_eq!(
        h.engine.pipeline_status(a).await.unwrap().state,
        PipelineState::Failed
    );
    // No orchestration event of one stream names the other's stages' failures.
    for (e, _) in orch(&h) {
        if let OrchestrationEvent::PipelineFailed { stream, .. } = e {
            assert_eq!(stream, a);
        }
    }
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn deadline_misses_are_typed_events_and_results_are_still_published() {
    let slow = echo("t.slow").with_faults(Faults {
        delay: Duration::from_millis(200),
        ..Faults::default()
    });
    let mut h = harness_with(audio_config(), utterances(), vec![slow]).await;
    h.grant_capture().await;
    let (mut reader, id) = open(&h).await;
    let mut spec = PipelineSpec::new(vec![
        recognition(),
        translation(Requirement::Required, "t.slow"),
    ]);
    spec.deadlines = DeadlinePolicy {
        translation_final_ms: Some(100),
        ..DeadlinePolicy::default()
    };
    let mut tr = en_es("t.slow");
    tr.partials = PartialPolicy::Off;
    spec.stages[1].config = Some(StageConfig::Translation(Box::new(tr)));
    h.engine.start_pipeline(id, spec).await.unwrap();
    feed(&h, &mut [&mut reader], 70).await;
    // The test clock is manual: advance it while the slow adapter works, so
    // the final translation is published 1 s (test time) after the audio.
    h.events
        .wait_for(WAIT, |e| {
            matches!(&e.payload, EventPayload::Speech(SpeechEvent::Transcript(r))
                if r.status == RevisionStatus::Final)
        })
        .await
        .unwrap();
    h.clock.advance(Duration::from_secs(3));
    wait_final_translation(&mut h, id, 0).await;
    match wait_orch(&mut h, |e| {
        matches!(e, OrchestrationEvent::DeadlineMissed { .. })
    })
    .await
    {
        OrchestrationEvent::DeadlineMissed {
            stage,
            utterance,
            latency_ms,
            budget_ms,
            ..
        } => {
            assert_eq!(
                (stage, utterance, budget_ms),
                (StageKind::Translation, 0, 100)
            );
            // Manual clock: audio time is anchored at the clock origin and
            // the utterance ended ~0.5 s into the audio; the clock is at 3 s.
            assert!((2_000..=3_000).contains(&latency_ms), "{latency_ms}");
        }
        _ => unreachable!(),
    }
    assert!(h.engine.metrics().deadline_misses >= 1);
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn repeated_lifecycles_and_shutdown_stay_bounded() {
    let mut h = harness_with(audio_config(), many_utterances(40), vec![echo("t.echo")]).await;
    h.grant_capture().await;
    let (mut reader, id) = open(&h).await;
    for round in 0..20 {
        h.engine.start_pipeline(id, full("t.echo")).await.unwrap();
        feed(&h, &mut [&mut reader], 10).await;
        if round % 2 == 0 {
            h.engine.restart_pipeline(id, None).await.unwrap();
        }
        let st = h.engine.stop_pipeline(id).await.unwrap();
        assert_eq!(st.state, PipelineState::Stopped);
    }
    let m = h.engine.metrics();
    assert_eq!(m.pipelines_started, 30);
    assert_eq!(m.pipelines_failed, 0);
    // Forwarders end with their workers.
    let deadline = std::time::Instant::now() + WAIT;
    while h.engine.metrics().worker_forwarders > 0 && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(h.engine.metrics().worker_forwarders, 0);
    let health = h.engine.health().await.unwrap();
    assert!(health.pipelines.len() <= 16);
    // A running pipeline is retired by shutdown, reported.
    h.engine.start_pipeline(id, full("t.echo")).await.unwrap();
    feed(&h, &mut [&mut reader], 10).await;
    let report = h.engine.shutdown().await.unwrap();
    assert!(report.clean, "{report:?}");
    let stopped = h
        .events
        .wait_for(WAIT, |e| {
            matches!(
                &e.payload,
                EventPayload::Orchestration(OrchestrationEvent::PipelineStopped {
                    reason: PipelineEndReason::Shutdown,
                    ..
                })
            )
        })
        .await;
    assert!(stopped.is_some());
}

#[tokio::test]
async fn an_analysis_and_recognition_pipeline_drains_with_the_hint_provider_last() {
    let mut h = harness_with(audio_config(), utterances(), vec![echo("t.echo")]).await;
    h.grant_capture().await;
    let (mut reader, id) = open(&h).await;
    let spec = PipelineSpec::new(vec![
        StageSpec::required(StageKind::Analysis),
        recognition(),
    ]);
    let st = h.engine.start_pipeline(id, spec).await.unwrap();
    assert_eq!(st.state, PipelineState::Running);
    feed(&h, &mut [&mut reader], 150).await;
    let fin = h
        .events
        .wait_for(WAIT, |e| {
            matches!(&e.payload, EventPayload::Speech(SpeechEvent::Transcript(r))
                if r.meta.utterance == 1 && r.status == RevisionStatus::Final)
        })
        .await;
    assert!(fin.is_some());
    // The fake backend's constant test signal triggers no activity, so the
    // real analyzer's timing is checked by `ce-orchestrate`'s replay test on
    // the Phase 3 fixture corpus; here: no finding on what was produced.
    let st = h.engine.pipeline_status(id).await.unwrap();
    let c = &st.counters;
    assert_eq!(
        (
            c.timeline_anomalies,
            c.re_anchors,
            c.unknown_timing,
            c.causal_violations
        ),
        (0, 0, 0, 0),
        "{c:?}"
    );
    // Drain order: recognition first, the hint provider (analysis) last.
    h.engine.stop_pipeline(id).await.unwrap();
    match wait_orch(&mut h, |e| {
        matches!(e, OrchestrationEvent::PipelineStopped { .. })
    })
    .await
    {
        OrchestrationEvent::PipelineStopped { outcomes, .. } => assert_eq!(
            outcomes,
            vec![
                (StageKind::Recognition, StopOutcome::Clean),
                (StageKind::Analysis, StopOutcome::Clean)
            ]
        ),
        _ => unreachable!(),
    }
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_required_stage_failing_at_start_rolls_back_the_started_stages() {
    let mut config = audio_config();
    config.translation.max_concurrent = 1;
    let mut h = harness_with(config, utterances(), vec![echo("t.echo")]).await;
    h.grant_capture().await;
    let (_ra, a) = open(&h).await;
    let (_rb, b) = open(&h).await;
    // A static configuration error is caught before anything starts.
    let mut no_target = en_es("t.echo");
    no_target.target = None;
    let spec = PipelineSpec::new(vec![
        recognition(),
        StageSpec::required(StageKind::Translation)
            .with_config(StageConfig::Translation(Box::new(no_target))),
    ]);
    assert_eq!(
        code(h.engine.start_pipeline(a, spec).await),
        "translation.no_target"
    );
    assert!(
        h.engine.recognition_status(a).await.is_err(),
        "nothing started"
    );
    // A run-time start failure (capacity, known only when starting) rolls
    // back the stages already started, downstream first.
    h.engine
        .start_translation(b, Some(en_es("t.echo")))
        .await
        .unwrap();
    assert_eq!(
        code(h.engine.start_pipeline(a, full("t.echo")).await),
        "translation.capacity_exhausted"
    );
    let rec = h.engine.recognition_status(a).await.unwrap();
    assert!(!rec.running, "recognition was rolled back");
    match wait_orch(&mut h, |e| {
        matches!(e, OrchestrationEvent::PipelineFailed { .. })
    })
    .await
    {
        OrchestrationEvent::PipelineFailed { stage, fault, .. } => {
            assert_eq!(stage, Some(StageKind::Translation));
            assert_eq!(fault, FaultClass::Overload);
        }
        _ => unreachable!(),
    }
    let st = h.engine.pipeline_status(a).await.unwrap();
    assert_eq!(st.state, PipelineState::Failed);
    assert_eq!(h.engine.metrics().pipelines_failed, 1);
    h.engine.shutdown().await.unwrap();
}
