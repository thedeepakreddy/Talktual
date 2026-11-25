//! Phase 7 audit corrections, verified through the public facade
//! (`phase-7-audit.md`): AUD-01 abandoned-worker cap, AUD-02 restart
//! validation before teardown, AUD-04 saturation, AUD-05 optional-stage
//! end, AUD-06 rollback outcomes. Fake audio, scripted ASR and scripted
//! translation adapters; waits are on observable conditions (events,
//! counters, gauges) with a bounded timeout, never on a fixed sleep as
//! the synchronization.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use ce_testkit::fake_audio::FakeDriver;
use common::*;
use communication_engine::contracts::config::EngineConfig;
use communication_engine::contracts::orchestration::{PipelineEndReason, StageState, StopOutcome};
use communication_engine::contracts::speech::{LanguageTag, SegmentationMode};
use communication_engine::contracts::translation::{PartialPolicy, SourceLanguagePolicy};
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

fn en_es(adapter: &str, partials: PartialPolicy) -> TranslationConfig {
    TranslationConfig {
        adapter: Some(adapter.into()),
        target: Some(LanguageTag::parse("es").unwrap()),
        source: SourceLanguagePolicy::Explicit {
            language: LanguageTag::parse("en").unwrap(),
        },
        partials,
        ..TranslationConfig::default()
    }
}

fn recognition() -> StageSpec {
    StageSpec::required(StageKind::Recognition)
        .with_config(StageConfig::Recognition(Box::new(continuous())))
}

fn translation(requirement: Requirement, adapter: &str) -> StageSpec {
    StageSpec {
        kind: StageKind::Translation,
        requirement,
        config: Some(StageConfig::Translation(Box::new(en_es(
            adapter,
            PartialPolicy::Full,
        )))),
    }
}

fn pipeline(adapter: &str, stop: StopMode) -> PipelineSpec {
    let mut s = PipelineSpec::new(vec![
        recognition(),
        translation(Requirement::Required, adapter),
    ]);
    s.stop = stop;
    s
}

fn echo(id: &str) -> ScriptedFactory {
    ScriptedFactory::echo(scripted_capabilities(id, &[("en", "es")]))
}

/// An adapter whose every call counts itself as entered, then blocks for
/// `hang` (longer than the stop bound, shorter than the stall detector).
fn hanging(id: &str, hang: Duration, entered: Arc<AtomicUsize>) -> ScriptedFactory {
    ScriptedFactory::new(scripted_capabilities(id, &[("en", "es")]), move |r| {
        entered.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(hang);
        AdapterOutcome::Translated {
            text: r.text.to_string(),
            score: None,
            detected_source: None,
        }
    })
}

/// An adapter that blocks while `gate` is closed.
type Gate = Arc<(Mutex<bool>, Condvar)>;

fn gated(id: &str, gate: Gate) -> ScriptedFactory {
    ScriptedFactory::new(scripted_capabilities(id, &[("en", "es")]), move |r| {
        let (lock, cv) = &*gate;
        let mut open = lock.lock().unwrap();
        while !*open {
            open = cv.wait(open).unwrap();
        }
        AdapterOutcome::Translated {
            text: format!("[es] {}", r.text),
            score: None,
            detected_source: None,
        }
    })
}

fn open_gate(gate: &Gate) {
    let (lock, cv) = &**gate;
    *lock.lock().unwrap() = true;
    cv.notify_all();
}

fn utterances(n: usize) -> ScriptedAsr {
    ScriptedAsr::new(
        (0..n)
            .map(|_| UtteranceScript::progressive("one two three four", 40).endpoint_at(200))
            .collect(),
    )
}

async fn harness(config: EngineConfig, n: usize, adapters: Vec<ScriptedFactory>) -> AudioHarness {
    audio_harness_custom(config, FakeDriver::Manual, move |mut b| {
        b = b.asr_adapter(utterances(n));
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

/// Wait (bounded by `WAIT`) until `cond` holds, polling an observable.
async fn until(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for: {what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

fn orch(h: &AudioHarness) -> Vec<OrchestrationEvent> {
    h.events
        .events()
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::Orchestration(o) => Some(o.clone()),
            _ => None,
        })
        .collect()
}

async fn wait_orch(
    h: &mut AudioHarness,
    pred: impl Fn(&OrchestrationEvent) -> bool,
) -> OrchestrationEvent {
    if let Some(e) = orch(h).into_iter().find(|e| pred(e)) {
        return e;
    }
    let e = h
        .events
        .wait_for(
            WAIT,
            |e| matches!(&e.payload, EventPayload::Orchestration(o) if pred(o)),
        )
        .await
        .expect("orchestration event");
    match &e.payload {
        EventPayload::Orchestration(o) => o.clone(),
        _ => unreachable!(),
    }
}

fn code<T: std::fmt::Debug>(r: Result<T, CeError>) -> String {
    r.unwrap_err().code().to_owned()
}

/// Start a pipeline whose translation hangs, wait until the adapter call
/// is in progress, then cancel-stop it: the translation worker is abandoned.
async fn abandon_one(
    h: &AudioHarness,
    r: &mut CaptureReader,
    id: AudioStreamId,
    entered: &Arc<AtomicUsize>,
) {
    let before = entered.load(Ordering::SeqCst);
    h.engine
        .start_pipeline(id, pipeline("t.hang", StopMode::Cancel))
        .await
        .unwrap();
    let mut fed = 0;
    while entered.load(Ordering::SeqCst) == before {
        feed(h, &mut [&mut *r], 5).await;
        fed += 5;
        assert!(fed < 400, "the hanging adapter was never called");
    }
    let st = h.engine.stop_pipeline(id).await.unwrap();
    let tr = st
        .stages
        .iter()
        .find(|s| s.kind == StageKind::Translation)
        .unwrap();
    assert_eq!(tr.stop, Some(StopOutcome::TimedOut));
}

fn cap_config(max_abandoned: u32) -> EngineConfig {
    let mut c = audio_config();
    c.orchestration.max_abandoned_workers = max_abandoned;
    c
}

// ------------------------------------------------------------------ AUD-01

#[tokio::test]
async fn aud01_the_abandoned_worker_cap_refuses_new_workers_and_recovers() {
    let entered = Arc::new(AtomicUsize::new(0));
    let hang = hanging("t.hang", Duration::from_millis(1_500), entered.clone());
    let h = harness(cap_config(2), 400, vec![hang, echo("t.echo")]).await;
    h.grant_capture().await;
    let (mut r, id) = open(&h).await;
    let (_r2, other) = open(&h).await;
    assert_eq!(h.engine.metrics().abandoned_workers, 0);
    abandon_one(&h, &mut r, id, &entered).await;
    abandon_one(&h, &mut r, id, &entered).await;
    until("gauge at 2", || h.engine.metrics().abandoned_workers == 2).await;
    // At the cap: every new stage worker is refused, typed, before it starts.
    assert_eq!(
        code(
            h.engine
                .start_pipeline(id, pipeline("t.echo", StopMode::Drain))
                .await
        ),
        "orchestration.abandoned_workers_exhausted"
    );
    assert_eq!(
        code(h.engine.start_recognition(other, Some(continuous())).await),
        "orchestration.abandoned_workers_exhausted"
    );
    assert_eq!(
        code(
            h.engine
                .start_translation(other, Some(en_es("t.echo", PartialPolicy::Off)))
                .await
        ),
        "orchestration.abandoned_workers_exhausted"
    );
    assert_eq!(
        code(h.engine.start_analysis(other, None).await),
        "orchestration.abandoned_workers_exhausted"
    );
    assert!(h.engine.metrics().starts_refused_abandoned >= 3);
    assert!(
        h.engine.recognition_status(other).await.is_err(),
        "nothing was started"
    );
    let threads_at_cap = h.engine.metrics().worker_forwarders;
    // Recovery: once the hung calls return, the threads exit and the
    // capacity returns (accounting follows the threads, not a counter).
    until("abandoned threads exited", || {
        h.engine.metrics().abandoned_workers == 0
    })
    .await;
    h.engine
        .start_pipeline(id, pipeline("t.echo", StopMode::Drain))
        .await
        .unwrap();
    assert!(h.engine.metrics().worker_forwarders <= threads_at_cap + 2);
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn aud01_concurrent_requests_cannot_exceed_the_cap_and_growth_stays_bounded() {
    let entered = Arc::new(AtomicUsize::new(0));
    let hang = hanging("t.hang", Duration::from_millis(3_000), entered.clone());
    let mut config = cap_config(2);
    config.translation.max_concurrent = 4;
    let h = Arc::new(harness(config, 400, vec![hang, echo("t.echo")]).await);
    h.grant_capture().await;
    let mut readers = Vec::new();
    let mut ids = Vec::new();
    for _ in 0..3 {
        let (r, id) = open(&h).await;
        readers.push(r);
        ids.push(id);
    }
    // Three pipelines whose translations are inside the hung call.
    for id in &ids {
        h.engine
            .start_pipeline(*id, pipeline("t.hang", StopMode::Cancel))
            .await
            .unwrap();
    }
    let mut fed = 0;
    while entered.load(Ordering::SeqCst) < 3 {
        for r in readers.iter_mut() {
            h.fake.tick(160);
            let _ = recv_block(r).await;
        }
        fed += 1;
        assert!(fed < 400);
    }
    // Stop all three concurrently: each is abandoned (running workers may
    // be abandoned past the cap: they existed before it was reached).
    let stops = ids.iter().map(|id| {
        let h = h.clone();
        let id = *id;
        async move { h.engine.stop_pipeline(id).await.unwrap() }
    });
    futures_join(stops).await;
    until("three abandoned", || {
        h.engine.metrics().abandoned_workers == 3
    })
    .await;
    // Twelve simultaneous start requests: none is admitted.
    let starts = (0..12).map(|i| {
        let h = h.clone();
        let id = ids[i % 3];
        async move {
            h.engine
                .start_pipeline(id, pipeline("t.echo", StopMode::Drain))
                .await
        }
    });
    let results = futures_join(starts).await;
    assert!(
        results.iter().all(
            |r| matches!(r, Err(e) if e.code() == "orchestration.abandoned_workers_exhausted")
        )
    );
    assert_eq!(
        h.engine.metrics().abandoned_workers,
        3,
        "no growth at the cap"
    );
    // Documented bound: abandoned ≤ cap − 1 + workers running when the cap
    // was reached (here 2 − 1 + 3 = 4; observed 3).
    assert!(h.engine.metrics().abandoned_workers <= 4);
    until("recovered", || h.engine.metrics().abandoned_workers == 0).await;
    for id in &ids {
        h.engine
            .start_pipeline(*id, pipeline("t.echo", StopMode::Drain))
            .await
            .unwrap();
    }
    let h = Arc::try_unwrap(h).ok().expect("sole owner");
    h.engine.shutdown().await.unwrap();
}

/// Join futures concurrently without an extra dependency (spawned tasks).
async fn futures_join<F, T>(futs: impl Iterator<Item = F>) -> Vec<T>
where
    F: std::future::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let handles: Vec<_> = futs.map(tokio::spawn).collect();
    let mut out = Vec::new();
    for h in handles {
        out.push(h.await.unwrap());
    }
    out
}

#[tokio::test]
async fn aud01_restart_and_reset_at_the_cap_keep_running_pipelines() {
    let entered = Arc::new(AtomicUsize::new(0));
    let hang = hanging("t.hang", Duration::from_millis(1_500), entered.clone());
    let h = harness(cap_config(1), 400, vec![hang, echo("t.echo")]).await;
    h.grant_capture().await;
    let (mut ra, a) = open(&h).await;
    let (mut rb, b) = open(&h).await;
    let healthy = h
        .engine
        .start_pipeline(b, pipeline("t.echo", StopMode::Drain))
        .await
        .unwrap();
    abandon_one(&h, &mut ra, a, &entered).await;
    until("at cap", || h.engine.metrics().abandoned_workers == 1).await;
    // Restart would need new workers: refused before the running pipeline
    // is touched (AUD-01 + AUD-02).
    assert_eq!(
        code(h.engine.restart_pipeline(b, None).await),
        "orchestration.abandoned_workers_exhausted"
    );
    let st = h.engine.pipeline_status(b).await.unwrap();
    assert_eq!(
        (st.state, st.epoch),
        (PipelineState::Running, healthy.epoch)
    );
    // Reset creates no worker: allowed at the cap.
    h.engine.reset_pipeline(b).await.unwrap();
    feed(&h, &mut [&mut rb], 20).await;
    assert_eq!(
        h.engine.pipeline_status(b).await.unwrap().state,
        PipelineState::Running
    );
    // Shutdown with an abandoned worker still hung completes within its bound.
    let t = Instant::now();
    h.engine.shutdown().await.unwrap();
    assert!(t.elapsed() < WAIT);
}

// ------------------------------------------------------------------ AUD-02

#[tokio::test]
async fn aud02_an_invalid_restart_leaves_the_running_pipeline_untouched() {
    let mut h = harness(audio_config(), 40, vec![echo("t.echo")]).await;
    h.grant_capture().await;
    let (mut r, id) = open(&h).await;
    let before = h
        .engine
        .start_pipeline(id, pipeline("t.echo", StopMode::Drain))
        .await
        .unwrap();
    feed(&h, &mut [&mut r], 10).await;
    let gens = |s: &PipelineStatus| s.stages.iter().map(|x| x.generation).collect::<Vec<_>>();
    let only_translation = PipelineSpec::new(vec![translation(Requirement::Required, "t.echo")]);
    let missing_adapter = pipeline("missing", StopMode::Drain);
    let mut bad_config = pipeline("t.echo", StopMode::Drain);
    bad_config.deadlines.transcript_final_ms = Some(0);
    let duplicate = PipelineSpec::new(vec![recognition(), recognition()]);
    let mut no_target = en_es("t.echo", PartialPolicy::Off);
    no_target.target = None;
    let missing_target = PipelineSpec::new(vec![
        recognition(),
        StageSpec::required(StageKind::Translation)
            .with_config(StageConfig::Translation(Box::new(no_target))),
    ]);
    let mut weak = pipeline("t.echo", StopMode::Drain);
    weak.stages[0].requirement = Requirement::Optional;
    let cases = [
        (only_translation, "orchestration.missing_dependency"),
        (missing_adapter, "translation.no_adapter"),
        (bad_config, "config.orchestration_invalid"),
        (duplicate, "orchestration.duplicate_stage"),
        (missing_target, "translation.no_target"),
        (weak, "orchestration.required_depends_on_optional"),
    ];
    for (spec, expected) in cases {
        assert_eq!(
            code(h.engine.restart_pipeline(id, Some(spec)).await),
            expected
        );
        let st = h.engine.pipeline_status(id).await.unwrap();
        assert_eq!(st.state, PipelineState::Running, "{expected}");
        assert_eq!(st.epoch, before.epoch, "{expected}: no new epoch");
        assert_eq!(gens(&st), gens(&before), "{expected}: same workers");
    }
    assert!(
        !orch(&h)
            .iter()
            .any(|e| matches!(e, OrchestrationEvent::PipelineStopped { .. })),
        "no lifecycle transition for an invalid restart"
    );
    // The pipeline still works.
    feed(&h, &mut [&mut r], 60).await;
    let fin = h
        .events
        .wait_for(WAIT, |e| {
            matches!(&e.payload, EventPayload::Translation(TranslationEvent::Result(r))
                if r.state == TranslationState::Final)
        })
        .await;
    assert!(fin.is_some());
    // A valid restart follows the normal lifecycle: stopped (restart), new epoch.
    let after = h.engine.restart_pipeline(id, None).await.unwrap();
    assert!(after.epoch > before.epoch);
    wait_orch(&mut h, |e| {
        matches!(
            e,
            OrchestrationEvent::PipelineStopped {
                reason: PipelineEndReason::Restart,
                ..
            }
        )
    })
    .await;
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn aud02_aud06_a_valid_replacement_that_fails_at_start_rolls_back_and_reports() {
    let mut config = audio_config();
    config.translation.max_concurrent = 1;
    let mut h = harness(config, 40, vec![echo("t.echo")]).await;
    h.grant_capture().await;
    let (_ra, a) = open(&h).await;
    let (_rb, b) = open(&h).await;
    h.engine
        .start_pipeline(a, PipelineSpec::new(vec![recognition()]))
        .await
        .unwrap();
    // The only translation slot is taken outside the pipeline.
    h.engine
        .start_translation(b, Some(en_es("t.echo", PartialPolicy::Off)))
        .await
        .unwrap();
    // Valid replacement (passes every pre-start check); its translation can
    // only fail when it starts (worker capacity).
    assert_eq!(
        code(
            h.engine
                .restart_pipeline(a, Some(pipeline("t.echo", StopMode::Drain)))
                .await
        ),
        "translation.capacity_exhausted"
    );
    let st = h.engine.pipeline_status(a).await.unwrap();
    assert_eq!(st.state, PipelineState::Failed);
    let rec = st
        .stages
        .iter()
        .find(|s| s.kind == StageKind::Recognition)
        .unwrap();
    assert_eq!(rec.stop, Some(StopOutcome::Clean), "rolled back, reported");
    match wait_orch(&mut h, |e| {
        matches!(
            e,
            OrchestrationEvent::PipelineStopped {
                reason: PipelineEndReason::Failed,
                ..
            }
        )
    })
    .await
    {
        OrchestrationEvent::PipelineStopped { outcomes, .. } => {
            assert_eq!(outcomes, vec![(StageKind::Recognition, StopOutcome::Clean)])
        }
        _ => unreachable!(),
    }
    assert!(!h.engine.recognition_status(a).await.unwrap().running);
    h.engine.shutdown().await.unwrap();
}

// ------------------------------------------------------------------ AUD-05

#[tokio::test]
async fn aud05_an_optional_stage_stopped_directly_degrades_the_pipeline() {
    let mut h = harness(audio_config(), 4, vec![echo("t.echo")]).await;
    h.grant_capture().await;
    let (mut r, id) = open(&h).await;
    h.engine
        .start_pipeline(
            id,
            PipelineSpec::new(vec![
                recognition(),
                translation(Requirement::Optional, "t.echo"),
            ]),
        )
        .await
        .unwrap();
    feed(&h, &mut [&mut r], 20).await;
    h.engine.stop_translation(id).await.unwrap();
    match wait_orch(&mut h, |e| {
        matches!(e, OrchestrationEvent::StageDegraded { .. })
    })
    .await
    {
        OrchestrationEvent::StageDegraded { stage, code, .. } => {
            assert_eq!(stage, StageKind::Translation);
            assert_eq!(code, "orchestration.stage_ended");
        }
        _ => unreachable!(),
    }
    let st = h.engine.pipeline_status(id).await.unwrap();
    assert_eq!(st.state, PipelineState::Degraded);
    let tr = st
        .stages
        .iter()
        .find(|s| s.kind == StageKind::Translation)
        .unwrap();
    assert_eq!(tr.state, StageState::Ended);
    // Stopping the pipeline normally emits no further degradation.
    h.engine.stop_pipeline(id).await.unwrap();
    let degraded = orch(&h)
        .iter()
        .filter(|e| matches!(e, OrchestrationEvent::StageDegraded { .. }))
        .count();
    assert_eq!(degraded, 1);
    h.engine.shutdown().await.unwrap();
}

// ------------------------------------------------------------------ AUD-04

/// Sustained load with one stage saturated: stream A's translation adapter
/// is blocked while both streams keep producing transcripts. Verifies the
/// bounded queue (coalescing, finals kept), that stream B keeps being
/// translated (isolation and fairness at the stage level), command
/// responsiveness, no growth of forwarders, and recovery and shutdown
/// after the saturation ends.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn aud04_a_saturated_stage_stays_bounded_and_isolated_and_recovers() {
    let gate: Gate = Arc::new((Mutex::new(false), Condvar::new()));
    let mut h = harness(
        audio_config(),
        400,
        vec![gated("t.gated", gate.clone()), echo("t.echo")],
    )
    .await;
    h.grant_capture().await;
    let (mut ra, a) = open(&h).await;
    let (mut rb, b) = open(&h).await;
    h.engine
        .start_pipeline(a, pipeline("t.gated", StopMode::Drain))
        .await
        .unwrap();
    h.engine
        .start_pipeline(b, pipeline("t.echo", StopMode::Drain))
        .await
        .unwrap();
    let forwarders = h.engine.metrics().worker_forwarders;
    // Sustained burst (about 2.4 s of audio per stream; finished well
    // within the 5 s stall bound of the blocked worker).
    let mut round_trips = Vec::new();
    for i in 0..240 {
        h.fake.tick(160);
        let _ = recv_block(&mut ra).await;
        let _ = recv_block(&mut rb).await;
        if i % 4 == 0 {
            let t = Instant::now();
            h.engine.pipeline_status(a).await.unwrap();
            round_trips.push(t.elapsed());
        }
    }
    // Saturated stage: the bounded queue coalesced superseded partials;
    // nothing was translated yet; the backlog stays within its bound.
    // Saturated stage: inputs wait in the worker's bounded queue (its
    // counters are published when the blocked call returns).
    let ta = h.engine.translation_status(a).await.unwrap();
    assert!(ta.running);
    assert!(
        ta.backlog > 0 && ta.backlog <= 64,
        "backlog {} (bound 2 × queue 32)",
        ta.backlog
    );
    assert_eq!(
        h.engine.metrics().translation_inputs_dropped,
        0,
        "no input refused: superseded partials are coalesced, finals kept"
    );
    // Stream B was translated meanwhile (isolation and fairness).
    let tb = h.engine.translation_status(b).await.unwrap();
    assert!(tb.counters.finals >= 2, "{:?}", tb.counters);
    // Commands stayed responsive (wall clock; generous bound for CI).
    let worst = round_trips.iter().max().unwrap();
    assert!(
        *worst < Duration::from_secs(1),
        "worst round trip {worst:?}"
    );
    // No growth of forwarders or pipelines under load.
    assert_eq!(h.engine.metrics().worker_forwarders, forwarders);
    // Recovery: open the gate; the queued finals are translated.
    open_gate(&gate);
    let fin = h
        .events
        .wait_for(WAIT, |e| {
            matches!(&e.payload, EventPayload::Translation(TranslationEvent::Result(r))
                if r.source.stream == a && r.state == TranslationState::Final)
        })
        .await;
    assert!(fin.is_some(), "stream A translated its queued finals");
    let ta = h.engine.translation_status(a).await.unwrap();
    assert!(ta.counters.coalesced > 0, "{:?}", ta.counters);
    assert!(ta.counters.finals >= 1, "{:?}", ta.counters);
    let st = h.engine.pipeline_status(a).await.unwrap();
    assert_eq!(st.counters.causal_violations, 0, "{:?}", st.counters);
    let report = h.engine.shutdown().await.unwrap();
    assert!(report.clean, "{report:?}");
}
