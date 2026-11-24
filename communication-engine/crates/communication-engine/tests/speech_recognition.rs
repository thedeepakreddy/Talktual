//! Phase 4 speech recognition through the public facade, with the fake
//! audio backend, scripted ASR adapters and scripted detectors: discovery,
//! lifecycle, start errors and limits, Phase 3 hint integration, privacy
//! classes, faults (panic, load failure and timeout, stall), stream end,
//! revocation, session close, shutdown, multi-stream, repeated lifecycle
//! and overload.

mod common;

use std::sync::atomic::Ordering::SeqCst;
use std::time::Duration;

use ce_testkit::fake_audio::FakeDriver;
use common::*;
use communication_engine::audio_intel::ScriptedVad;
use communication_engine::contracts::config::EngineConfig;
use communication_engine::contracts::intel::AnalysisConfig;
use communication_engine::contracts::speech::{
    FinalReason, HintSource, RecognitionEndReason, ResetReason, SegmentationMode,
};
use communication_engine::prelude::*;
use communication_engine::speech::fake::{
    LoadBehavior, ScriptedFactory, ScriptedFault, UtteranceScript,
};

fn capture_16k_mono() -> StreamRequest {
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

fn script() -> ScriptedFactory {
    ScriptedFactory::new(vec![
        UtteranceScript::progressive("hello world again", 100).endpoint_at(400),
    ])
}

/// Not about resources: no throttling, deep queues.
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

async fn speech_harness(factories: Vec<ScriptedFactory>) -> AudioHarness {
    speech_harness_with(audio_config(), factories).await
}

async fn speech_harness_with(
    config: EngineConfig,
    factories: Vec<ScriptedFactory>,
) -> AudioHarness {
    audio_harness_custom(config, FakeDriver::Manual, move |mut b| {
        for f in factories {
            b = b.asr_adapter(f);
        }
        b
    })
    .await
}

fn speech_events(events: &ce_testkit::EventRecorder) -> Vec<(SpeechEvent, DataClass)> {
    events
        .events()
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::Speech(s) => Some((s.clone(), e.privacy)),
            _ => None,
        })
        .collect()
}

/// The first matching speech event, already recorded or arriving within
/// `WAIT`.
async fn wait_speech(h: &mut AudioHarness, pred: impl Fn(&SpeechEvent) -> bool) -> SpeechEvent {
    if let Some((e, _)) = speech_events(&h.events).into_iter().find(|(e, _)| pred(e)) {
        return e;
    }
    let e = h
        .events
        .wait_for(
            WAIT,
            |e| matches!(&e.payload, EventPayload::Speech(s) if pred(s)),
        )
        .await;
    let Some(e) = e else {
        let kinds: Vec<&str> = speech_events(&h.events)
            .iter()
            .map(|(e, _)| e.kind())
            .collect();
        panic!("expected speech event; got {kinds:?}");
    };
    match &e.payload {
        EventPayload::Speech(s) => s.clone(),
        _ => unreachable!(),
    }
}

fn is_final(e: &SpeechEvent, stream: AudioStreamId) -> bool {
    matches!(e, SpeechEvent::Transcript(r) if r.meta.stream == stream && r.status == RevisionStatus::Final)
}

fn stopped(e: &SpeechEvent, stream: AudioStreamId) -> Option<RecognitionEndReason> {
    match e {
        SpeechEvent::Stopped {
            stream: s, reason, ..
        } if *s == stream => Some(*reason),
        _ => None,
    }
}

async fn open_capture(h: &AudioHarness) -> (CaptureReader, AudioStreamId) {
    let reader = h
        .engine
        .open_capture(h.session.id, capture_16k_mono())
        .await
        .unwrap();
    let id = reader.info().id;
    h.engine.start_audio(id).await.unwrap();
    (reader, id)
}

/// Tick `blocks` 10 ms blocks, keeping every host reader drained.
async fn feed(h: &AudioHarness, readers: &mut [&mut CaptureReader], blocks: usize) {
    for _ in 0..blocks {
        h.fake.tick(160);
        for r in readers.iter_mut() {
            let _ = recv_block(r).await;
        }
    }
}

#[tokio::test]
async fn recognition_lifecycle_through_the_facade() {
    let f = script();
    let stats = f.stats();
    let mut h = speech_harness(vec![f]).await;
    let caps = h.engine.recognizers().await.unwrap();
    assert_eq!(caps.len(), 1);
    assert_eq!(caps[0].adapter, "ce.asr.scripted");
    assert_eq!(stats.created.load(SeqCst), 0, "discovery creates nothing");
    h.grant_capture().await;
    let (mut reader, id) = open_capture(&h).await;
    let st = h
        .engine
        .start_recognition(id, Some(continuous()))
        .await
        .unwrap();
    assert!(st.running && !st.capabilities.code_switching);
    assert_eq!(st.hints, HintSource::Continuous);
    wait_speech(&mut h, |e| matches!(e, SpeechEvent::ModelReady { .. })).await;
    feed(&h, &mut [&mut reader], 60).await;
    let fin = wait_speech(&mut h, |e| is_final(e, id)).await;
    let SpeechEvent::Transcript(r) = fin else {
        unreachable!()
    };
    assert_eq!(r.text.committed, "hello world again");
    assert_eq!(r.final_reason, Some(FinalReason::AdapterEndpoint));
    assert_eq!(r.meta.frame_range.1 - r.meta.frame_range.0, 6_400);
    // Events are in the stream's session with their privacy class.
    for e in h.events.events() {
        if let EventPayload::Speech(s) = &e.payload {
            assert_eq!(e.session_id, Some(h.session.id));
            assert_eq!(e.privacy, s.data_class());
        }
    }
    assert!(
        speech_events(&h.events)
            .iter()
            .any(|(e, c)| matches!(e, SpeechEvent::Transcript(_)) && *c == DataClass::Content)
    );
    let st = h.engine.recognition_status(id).await.unwrap();
    assert!(st.counters.finals >= 1 && st.loaded);
    assert_eq!(h.engine.metrics().recognitions_started, 1);
    let health = h.engine.health().await.unwrap();
    assert_eq!(health.recognitions.len(), 1);
    let fin = h.engine.stop_recognition(id).await.unwrap();
    assert_eq!(fin.ended, Some(RecognitionEndReason::Stopped));
    assert!(!fin.running);
    // Stop again / query after the end return the final status.
    assert_eq!(
        h.engine.stop_recognition(id).await.unwrap().ended,
        Some(RecognitionEndReason::Stopped)
    );
    assert!(h.engine.recognition_status(id).await.is_ok());
    assert_eq!(stats.live.load(SeqCst), 0, "model released after stop");
    let report = h.engine.shutdown().await.unwrap();
    assert!(report.clean);
}

#[tokio::test]
async fn start_errors_and_concurrency_limits() {
    let mut config = audio_config();
    config.speech.max_concurrent_recognitions = 2;
    let limited = ScriptedFactory::new(vec![]).with_capabilities(|c| {
        c.adapter = "ce.asr.limited".into();
        c.max_concurrent_streams = Some(1);
    });
    let h = speech_harness_with(config, vec![script(), limited]).await;
    h.grant_capture().await;
    let (_r1, a) = open_capture(&h).await;
    let (_r2, b) = open_capture(&h).await;
    let (_r3, c) = open_capture(&h).await;
    let code = |r: Result<RecognitionStatus, CeError>| r.unwrap_err().code().to_owned();
    let unknown = AudioStreamId::from_uuid(uuid::Uuid::from_u128(77));
    assert_eq!(
        code(
            h.engine
                .start_recognition(unknown, Some(continuous()))
                .await
        ),
        "audio.unknown_stream"
    );
    assert_eq!(
        code(h.engine.start_recognition(a, None).await),
        "speech.analysis_required"
    );
    let mut other = continuous();
    other.adapter = Some("nope".into());
    assert_eq!(
        code(h.engine.start_recognition(a, Some(other)).await),
        "speech.no_recognizer"
    );
    let mut bad = continuous();
    bad.chunk_ms = 1;
    assert_eq!(
        code(h.engine.start_recognition(a, Some(bad)).await),
        "config.recognition_invalid"
    );
    let mut lim = continuous();
    lim.adapter = Some("ce.asr.limited".into());
    h.engine
        .start_recognition(a, Some(lim.clone()))
        .await
        .unwrap();
    assert_eq!(
        code(h.engine.start_recognition(a, Some(continuous())).await),
        "speech.already_running"
    );
    assert_eq!(
        code(h.engine.start_recognition(b, Some(lim)).await),
        "speech.capacity_exhausted",
        "per-adapter limit"
    );
    h.engine
        .start_recognition(b, Some(continuous()))
        .await
        .unwrap();
    assert_eq!(
        code(h.engine.start_recognition(c, Some(continuous())).await),
        "speech.capacity_exhausted",
        "engine-wide limit"
    );
    assert_eq!(
        code(h.engine.reset_recognition(c).await),
        "speech.not_found"
    );
    // Without any adapter registered.
    let h2 = speech_harness(vec![]).await;
    h2.grant_capture().await;
    let (_r, x) = open_capture(&h2).await;
    assert_eq!(
        code(h2.engine.start_recognition(x, Some(continuous())).await),
        "speech.no_recognizer"
    );
    assert!(h2.engine.recognizers().await.unwrap().is_empty());
}

#[tokio::test]
async fn phase3_activity_hints_delimit_utterances_end_to_end() {
    let f = ScriptedFactory::new(vec![UtteranceScript::progressive(
        "could you tell me where the train station is",
        60,
    )]);
    let mut h = speech_harness(vec![f]).await;
    h.grant_capture().await;
    let (mut reader, id) = open_capture(&h).await;
    let mut acfg = AnalysisConfig::default();
    acfg.budget.degrade = false;
    acfg.budget.queue_blocks = 4_000;
    // Activity for hops [20, 80) (0.2 s – 0.8 s), silence after.
    h.engine
        .start_analysis_with(
            id,
            Some(acfg),
            Box::new(ScriptedVad::new(|i| {
                if (20..80).contains(&i) { 0.95 } else { 0.02 }
            })),
        )
        .await
        .unwrap();
    let mut rcfg = RecognitionConfig {
        partial_interval_ms: 50,
        ..RecognitionConfig::default()
    };
    rcfg.budget.degrade = false;
    rcfg.budget.queue_blocks = 4_000;
    let st = h.engine.start_recognition(id, Some(rcfg)).await.unwrap();
    assert_eq!(st.hints, HintSource::Activity);
    feed(&h, &mut [&mut reader], 250).await;
    let fin = wait_speech(&mut h, |e| is_final(e, id)).await;
    let SpeechEvent::Transcript(r) = fin else {
        unreachable!()
    };
    assert_eq!(r.final_reason, Some(FinalReason::SegmentEnd));
    assert_eq!(r.meta.hints, HintSource::Activity);
    let (f0, f1) = r.meta.frame_range;
    // The onset is at ≈ 3200 (hop 20); the default 500 ms lead-in reaches
    // back to the stream start.
    assert_eq!(f0, 0, "starts lead_in_ms before the onset: {f0}");
    assert!(f1 >= 80 * 160, "covers the activity: {f1}");
    // Exactly one utterance: nothing is recognized in silence.
    let st = h.engine.recognition_status(id).await.unwrap();
    assert_eq!(st.counters.utterances, 1);
    assert!(st.counters.final_delay_ms.count >= 1);
    // Stopping the analysis falls back to continuous delimitation.
    h.engine.stop_analysis(id).await.unwrap();
    wait_speech(&mut h, |e| {
        matches!(
            e,
            SpeechEvent::HintsChanged {
                hints: HintSource::Continuous,
                ..
            }
        )
    })
    .await;
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn an_adapter_panic_is_contained_and_other_streams_continue() {
    let bad = script().with_fault(ScriptedFault::Panic {
        utterance: 0,
        at_ms: 200,
    });
    let good = script().with_capabilities(|c| c.adapter = "ce.asr.good".into());
    let mut h = speech_harness(vec![bad, good]).await;
    h.grant_capture().await;
    let (mut ra, a) = open_capture(&h).await;
    let (mut rb, b) = open_capture(&h).await;
    h.engine
        .start_recognition(a, Some(continuous()))
        .await
        .unwrap();
    let mut g = continuous();
    g.adapter = Some("ce.asr.good".into());
    h.engine.start_recognition(b, Some(g)).await.unwrap();
    feed(&h, &mut [&mut ra, &mut rb], 60).await;
    let failed = wait_speech(
        &mut h,
        |e| matches!(e, SpeechEvent::Failed { stream, .. } if *stream == a),
    )
    .await;
    let SpeechEvent::Failed { error, .. } = failed else {
        unreachable!()
    };
    assert_eq!(error.code, "speech.worker_panicked");
    let r = wait_speech(&mut h, |e| stopped(e, a).is_some()).await;
    assert_eq!(stopped(&r, a), Some(RecognitionEndReason::Failed));
    wait_speech(&mut h, |e| is_final(e, b)).await;
    // Capture and the other recognition carry on.
    feed(&h, &mut [&mut ra, &mut rb], 60).await;
    assert!(h.engine.recognition_status(b).await.unwrap().running);
    assert!(h.engine.metrics().recognition_failures >= 1);
    assert!(h.engine.shutdown().await.unwrap().clean);
}

#[tokio::test]
async fn load_failures_and_load_timeouts_are_reported() {
    let failing = script().with_load(LoadBehavior {
        fail: true,
        ..Default::default()
    });
    let slow = script()
        .with_capabilities(|c| c.adapter = "ce.asr.slow".into())
        .with_load(LoadBehavior {
            delay_ms: 1_500,
            ..Default::default()
        });
    let panicking = script()
        .with_capabilities(|c| c.adapter = "ce.asr.panicky".into())
        .with_load(LoadBehavior {
            panic: true,
            ..Default::default()
        });
    let mut h = speech_harness(vec![failing, slow, panicking]).await;
    h.grant_capture().await;
    let (_ra, a) = open_capture(&h).await;
    let (_rb, b) = open_capture(&h).await;
    let (_rc, c) = open_capture(&h).await;
    h.engine
        .start_recognition(a, Some(continuous()))
        .await
        .unwrap();
    let r = wait_speech(&mut h, |e| stopped(e, a).is_some()).await;
    assert_eq!(stopped(&r, a), Some(RecognitionEndReason::LoadFailed));
    let mut s = continuous();
    s.adapter = Some("ce.asr.slow".into());
    s.budget.load_timeout_ms = 200;
    h.engine.start_recognition(b, Some(s)).await.unwrap();
    let f = wait_speech(
        &mut h,
        |e| matches!(e, SpeechEvent::Failed { stream, .. } if *stream == b),
    )
    .await;
    let SpeechEvent::Failed { error, .. } = f else {
        unreachable!()
    };
    assert_eq!(error.code, "speech.load_timeout");
    let r = wait_speech(&mut h, |e| stopped(e, b).is_some()).await;
    assert_eq!(stopped(&r, b), Some(RecognitionEndReason::LoadFailed));
    let mut p = continuous();
    p.adapter = Some("ce.asr.panicky".into());
    h.engine.start_recognition(c, Some(p)).await.unwrap();
    let r = wait_speech(&mut h, |e| stopped(e, c).is_some()).await;
    assert_eq!(stopped(&r, c), Some(RecognitionEndReason::LoadFailed));
    // The late loader sends nothing after being abandoned.
    tokio::time::sleep(Duration::from_millis(1_600)).await;
    let after: Vec<_> = speech_events(&h.events)
        .into_iter()
        .filter(|(e, _)| e.stream() == b && matches!(e, SpeechEvent::ModelReady { .. }))
        .collect();
    assert!(after.is_empty());
    assert_eq!(h.engine.metrics().recognition_failures, 3);
    assert!(h.engine.shutdown().await.unwrap().clean);
}

#[tokio::test]
async fn a_stalled_adapter_is_detected_and_abandoned() {
    let f = script().with_fault(ScriptedFault::Stall {
        utterance: 0,
        at_ms: 100,
        ms: 1_500,
    });
    let mut h = speech_harness(vec![f]).await;
    h.grant_capture().await;
    let (mut reader, id) = open_capture(&h).await;
    let mut cfg = continuous();
    cfg.budget.stall_timeout_ms = 200;
    h.engine.start_recognition(id, Some(cfg)).await.unwrap();
    wait_speech(&mut h, |e| matches!(e, SpeechEvent::ModelReady { .. })).await;
    feed(&h, &mut [&mut reader], 20).await;
    let f = wait_speech(&mut h, |e| matches!(e, SpeechEvent::Failed { .. })).await;
    let SpeechEvent::Failed { error, .. } = f else {
        unreachable!()
    };
    assert_eq!(error.code, "speech.stalled");
    let r = wait_speech(&mut h, |e| stopped(e, id).is_some()).await;
    assert_eq!(stopped(&r, id), Some(RecognitionEndReason::Failed));
    // Capture keeps flowing while the worker is stuck.
    feed(&h, &mut [&mut reader], 50).await;
    tokio::time::sleep(Duration::from_millis(1_600)).await;
    assert!(h.engine.shutdown().await.unwrap().clean);
}

#[tokio::test]
async fn revocation_stream_close_reset_session_close_and_shutdown() {
    let f = ScriptedFactory::new(vec![UtteranceScript::progressive("a b c d e f g h", 100)]);
    let mut h = speech_harness(vec![f]).await;
    h.grant_capture().await;
    let (mut ra, a) = open_capture(&h).await;
    let (mut rb, b) = open_capture(&h).await;
    let (mut rc, c) = open_capture(&h).await;
    for s in [a, b, c] {
        h.engine
            .start_recognition(s, Some(continuous()))
            .await
            .unwrap();
    }
    feed(&h, &mut [&mut ra, &mut rb, &mut rc], 30).await;
    // Reset: the open utterance is cancelled; recognition continues.
    h.engine.reset_recognition(a).await.unwrap();
    wait_speech(&mut h, |e| {
        matches!(e, SpeechEvent::Reset { stream, reason: ResetReason::Requested } if *stream == a)
    })
    .await;
    wait_speech(&mut h, |e| {
        matches!(e, SpeechEvent::Transcript(r) if r.meta.stream == a && r.status == RevisionStatus::Cancelled)
    })
    .await;
    // Closing a stream finalizes its open utterance as StreamEnd.
    h.engine.close_audio(b).await.unwrap();
    let r = wait_speech(&mut h, |e| stopped(e, b).is_some()).await;
    assert_eq!(stopped(&r, b), Some(RecognitionEndReason::StreamClosed));
    assert!(speech_events(&h.events).iter().any(|(e, _)| matches!(
        e,
        SpeechEvent::Transcript(r) if r.meta.stream == b && r.final_reason == Some(FinalReason::StreamEnd)
    )));
    // Revocation discards: the utterance is cancelled, never finalized.
    h.engine
        .update_consent(
            h.session.id,
            ConsentPurpose::Capture,
            ConsentState::Revoked { at: h.now() },
        )
        .await
        .unwrap();
    let r = wait_speech(&mut h, |e| stopped(e, c).is_some()).await;
    assert_eq!(stopped(&r, c), Some(RecognitionEndReason::Revoked));
    assert!(!speech_events(&h.events).iter().any(|(e, _)| matches!(
        e,
        SpeechEvent::Transcript(r) if r.meta.stream == c && r.status == RevisionStatus::Final
    )));
    drop((ra, rb, rc));

    // Session close stops recognitions before releasing streams; shutdown
    // stops the rest with an explicit reason.
    let f = ScriptedFactory::new(vec![UtteranceScript::progressive("x y", 100)]);
    let mut h = speech_harness(vec![f]).await;
    h.grant_capture().await;
    let (mut r1, s1) = open_capture(&h).await;
    h.engine
        .start_recognition(s1, Some(continuous()))
        .await
        .unwrap();
    feed(&h, &mut [&mut r1], 10).await;
    let s2_session = h
        .engine
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    h.engine
        .update_consent(
            s2_session.id,
            ConsentPurpose::Capture,
            ConsentState::Granted { at: h.now() },
        )
        .await
        .unwrap();
    let r2 = h
        .engine
        .open_capture(s2_session.id, capture_16k_mono())
        .await
        .unwrap();
    let s2 = r2.info().id;
    h.engine.start_audio(s2).await.unwrap();
    h.engine
        .start_recognition(s2, Some(continuous()))
        .await
        .unwrap();
    h.engine.close_session(s2_session.id).await.unwrap();
    let r = wait_speech(&mut h, |e| stopped(e, s2).is_some()).await;
    assert_eq!(stopped(&r, s2), Some(RecognitionEndReason::StreamClosed));
    let report = h.engine.shutdown().await.unwrap();
    assert!(report.clean);
    h.events.drain_until_closed(WAIT).await;
    let reasons: Vec<_> = speech_events(&h.events)
        .iter()
        .filter_map(|(e, _)| stopped(e, s1))
        .collect();
    assert_eq!(reasons, vec![RecognitionEndReason::Shutdown]);
    assert!(speech_events(&h.events).iter().any(|(e, _)| matches!(
        e,
        SpeechEvent::Transcript(r) if r.meta.stream == s1 && r.final_reason == Some(FinalReason::Stopped)
    )));
}

#[tokio::test]
async fn multi_stream_recognition_and_repeated_lifecycle_do_not_leak() {
    let f = script();
    let stats = f.stats();
    let mut h = speech_harness(vec![f]).await;
    h.grant_capture().await;
    let mut readers = Vec::new();
    let mut ids = Vec::new();
    for _ in 0..4 {
        let (r, id) = open_capture(&h).await;
        h.engine
            .start_recognition(id, Some(continuous()))
            .await
            .unwrap();
        readers.push(r);
        ids.push(id);
    }
    {
        let mut refs: Vec<&mut CaptureReader> = readers.iter_mut().collect();
        feed(&h, &mut refs, 60).await;
    }
    for id in &ids {
        wait_speech(&mut h, |e| is_final(e, *id)).await;
    }
    assert!(stats.reuses.load(SeqCst) >= 3, "one loaded model shared");
    for id in &ids {
        h.engine.stop_recognition(*id).await.unwrap();
    }
    // Twenty start/stop cycles on one stream.
    let id = ids[0];
    for _ in 0..20 {
        h.engine
            .start_recognition(id, Some(continuous()))
            .await
            .unwrap();
        feed(&h, &mut [&mut readers[0]], 3).await;
        let st = h.engine.stop_recognition(id).await.unwrap();
        assert_eq!(st.ended, Some(RecognitionEndReason::Stopped));
    }
    let health = h.engine.health().await.unwrap();
    assert!(
        health.recognitions.len() <= 16,
        "finished statuses are bounded"
    );
    assert!(health.recognitions.iter().all(|s| !s.running));
    assert_eq!(stats.live.load(SeqCst), 0, "every adapter released");
    assert_eq!(h.engine.metrics().recognitions_started, 24);
    assert!(h.engine.shutdown().await.unwrap().clean);
}

#[tokio::test]
async fn overload_drops_recognizer_audio_without_disturbing_capture() {
    let f = script().with_fault(ScriptedFault::Stall {
        utterance: 0,
        at_ms: 100,
        ms: 600,
    });
    let mut h = speech_harness(vec![f]).await;
    h.grant_capture().await;
    let (mut reader, id) = open_capture(&h).await;
    let mut cfg = continuous();
    cfg.budget.queue_blocks = 8;
    h.engine.start_recognition(id, Some(cfg)).await.unwrap();
    wait_speech(&mut h, |e| matches!(e, SpeechEvent::ModelReady { .. })).await;
    let t0 = std::time::Instant::now();
    feed(&h, &mut [&mut reader], 100).await;
    assert!(
        t0.elapsed() < Duration::from_millis(3_000),
        "capture delivery is not delayed by a stuck recognizer"
    );
    // After the stall the recognizer drains its 8 queued blocks; the next
    // block reveals the dropped ones as a sequence gap.
    tokio::time::sleep(Duration::from_millis(700)).await;
    feed(&h, &mut [&mut reader], 20).await;
    wait_speech(&mut h, |e| matches!(e, SpeechEvent::QueuePressure { .. })).await;
    wait_speech(&mut h, |e| matches!(e, SpeechEvent::Discontinuity { .. })).await;
    let st = h.engine.recognition_status(id).await.unwrap();
    assert!(st.counters.tap_dropped_blocks > 0);
    assert!(st.running);
    assert!(h.engine.shutdown().await.unwrap().clean);
}
