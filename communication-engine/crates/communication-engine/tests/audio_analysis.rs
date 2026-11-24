//! Phase 3 audio intelligence through the public facade, with the fake
//! audio backend and scripted detectors: lifecycle, event delivery, stream
//! end (close, revoke, invalidation), failure isolation, overload,
//! stalls and shutdown.

mod common;

use std::time::Duration;

use ce_testkit::fake_audio::FakeDriver;
use common::*;
use communication_engine::audio_intel::ScriptedVad;
use communication_engine::contracts::intel::{
    ActivityState, AnalysisConfig, AnalysisEndReason, AnalysisEvent, DegradedMode,
    SegmentCloseReason,
};
use communication_engine::prelude::*;

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

fn analysis_events(h: &AudioHarness) -> Vec<AnalysisEvent> {
    h.events
        .events()
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::Analysis(a) => Some(a.clone()),
            _ => None,
        })
        .collect()
}

async fn wait_analysis(
    h: &mut AudioHarness,
    pred: impl Fn(&AnalysisEvent) -> bool,
) -> AnalysisEvent {
    let e = h
        .events
        .wait_for(
            WAIT,
            |e| matches!(&e.payload, EventPayload::Analysis(a) if pred(a)),
        )
        .await;
    let Some(e) = e else {
        let kinds: Vec<String> = analysis_events(h)
            .iter()
            .map(|e| match e {
                AnalysisEvent::DegradedModeChanged {
                    to,
                    backlog,
                    load_permille,
                    ..
                } => {
                    format!("degraded:{to:?}:{backlog}:{load_permille}")
                }
                other => other.kind().to_string(),
            })
            .collect();
        panic!(
            "expected analysis event; lagged {}; got {kinds:?}",
            h.events.lagged()
        );
    };
    match &e.payload {
        EventPayload::Analysis(a) => a.clone(),
        _ => unreachable!(),
    }
}

/// For tests that are not about resources: the Manual fake driver delivers
/// audio faster than real time and other tests busy-spin, so the
/// (wall-clock) resource governor would legitimately degrade the analysis
/// on a loaded test machine. Turn degradation off; keep a deep tap.
fn functional() -> AnalysisConfig {
    let mut cfg = AnalysisConfig::default();
    cfg.budget.degrade = false;
    cfg.budget.queue_blocks = 4_000;
    cfg
}

/// Activity for hops [20, 80), silence otherwise (10 ms hops).
fn burst() -> ScriptedVad {
    ScriptedVad::new(|i| if (20..80).contains(&i) { 0.95 } else { 0.02 })
}

async fn running_capture(h: &mut AudioHarness) -> (CaptureReader, AudioStreamId) {
    h.grant_capture().await;
    let reader = h
        .engine
        .open_capture(h.session.id, capture_16k_mono())
        .await
        .unwrap();
    let id = reader.info().id;
    h.engine.start_audio(id).await.unwrap();
    (reader, id)
}

/// Tick `blocks` 10 ms blocks, keeping the host reader drained.
async fn feed(h: &AudioHarness, reader: &mut CaptureReader, blocks: usize) -> usize {
    let mut got = 0;
    for _ in 0..blocks {
        h.fake.tick(160);
        got += 1;
        let _ = recv_block(reader).await;
    }
    got
}

#[tokio::test]
async fn analysis_reports_activity_segments_and_endpoints_without_disturbing_capture() {
    let mut h = audio_harness().await;
    let (mut reader, id) = running_capture(&mut h).await;
    let status = h
        .engine
        .start_analysis_with(id, Some(functional()), Box::new(burst()))
        .await
        .unwrap();
    assert!(status.running);
    assert_eq!(status.algorithm.vad, "ce.vad.scripted");
    assert!(!status.capabilities.overlap_detection);

    assert_eq!(
        feed(&h, &mut reader, 200).await,
        200,
        "the host still receives every block"
    );
    let ended = wait_analysis(&mut h, |e| matches!(e, AnalysisEvent::SegmentEnded { .. })).await;
    let AnalysisEvent::SegmentEnded {
        meta,
        reason,
        duration_ms,
        segment,
        ..
    } = ended
    else {
        unreachable!()
    };
    assert_eq!(
        (segment, reason, duration_ms),
        (0, SegmentCloseReason::SilencePolicy, 600)
    );
    assert_eq!(meta.stream, id);
    assert_eq!(meta.frame_range, (20 * 160, 80 * 160));
    assert_eq!(meta.mode, DegradedMode::Full);

    let final_status = h.engine.stop_analysis(id).await.unwrap();
    assert!(!final_status.running);
    assert_eq!(final_status.ended, Some(AnalysisEndReason::Stopped));
    assert_eq!(final_status.counters.segments, 1);
    assert!(final_status.counters.blocks >= 150);
    wait_analysis(&mut h, |e| matches!(e, AnalysisEvent::Stopped { .. })).await;

    let kinds: Vec<&str> = analysis_events(&h)
        .iter()
        .map(AnalysisEvent::kind)
        .collect();
    let first = |k: &str| {
        kinds
            .iter()
            .position(|x| *x == k)
            .unwrap_or_else(|| panic!("no {k} in {kinds:?}"))
    };
    assert_eq!(kinds[0], "started");
    assert!(first("activity_changed") < first("segment_started"));
    assert!(first("segment_started") < first("endpoint_candidate"));
    assert!(first("endpoint_candidate") < first("segment_ended"));
    assert_eq!(*kinds.last().unwrap(), "stopped");
    // Every analysis event is in the stream's session and in the Analysis topic.
    for e in h
        .events
        .events()
        .iter()
        .filter(|e| matches!(e.payload, EventPayload::Analysis(_)))
    {
        assert_eq!(e.session_id, Some(h.session.id));
        assert_eq!(e.payload.topic(), EventTopic::Analysis);
        assert_eq!(e.privacy, DataClass::Operational);
    }
    let health = h.engine.health().await.unwrap();
    assert_eq!(health.analyses.len(), 1);
    assert_eq!(health.metrics.analyses_started, 1);
    // Still capturing after analysis stopped.
    assert_eq!(feed(&h, &mut reader, 3).await, 3);
}

#[tokio::test]
async fn invalid_requests_are_rejected_with_stable_codes() {
    let mut h = audio_harness().await;
    let (_reader, id) = running_capture(&mut h).await;
    let playback = h
        .engine
        .open_playback(h.session.id, StreamRequest::playback())
        .await
        .unwrap();
    let code = |r: Result<_, CeError>| r.unwrap_err().code().to_string();
    assert_eq!(
        code(h.engine.start_analysis(playback.info().id, None).await),
        "analysis.not_capture"
    );
    let bad = AnalysisConfig {
        hop_ms: 7,
        ..AnalysisConfig::default()
    };
    assert_eq!(
        code(h.engine.start_analysis(id, Some(bad)).await),
        "config.analysis_invalid"
    );
    assert_eq!(
        code(h.engine.analysis_status(id).await),
        "analysis.not_found"
    );
    h.engine.start_analysis(id, None).await.unwrap();
    assert_eq!(
        code(h.engine.start_analysis(id, None).await),
        "analysis.already_running"
    );
    let s = h.engine.analysis_status(id).await.unwrap();
    assert_eq!(s.algorithm.vad, "ce.vad.spectral", "the default detector");
    h.engine.close_audio(id).await.unwrap();
    let stopped = wait_analysis(&mut h, |e| matches!(e, AnalysisEvent::Stopped { .. })).await;
    assert!(matches!(
        stopped,
        AnalysisEvent::Stopped {
            reason: AnalysisEndReason::StreamClosed,
            ..
        }
    ));
    assert_eq!(
        code(h.engine.start_analysis(id, None).await),
        "audio.unknown_stream"
    );
}

#[tokio::test]
async fn revocation_ends_analysis_and_discards_queued_audio() {
    let mut h = audio_harness().await;
    let (mut reader, id) = running_capture(&mut h).await;
    h.engine
        .start_analysis_with(
            id,
            Some(functional()),
            Box::new(ScriptedVad::new(|_| 0.95).with_delay(Duration::from_millis(2))),
        )
        .await
        .unwrap();
    feed(&h, &mut reader, 30).await;
    wait_analysis(&mut h, |e| {
        matches!(e, AnalysisEvent::SegmentStarted { .. })
    })
    .await;
    h.engine
        .update_consent(
            h.session.id,
            ConsentPurpose::Capture,
            ConsentState::Revoked { at: h.now() },
        )
        .await
        .unwrap();
    let stopped = wait_analysis(&mut h, |e| matches!(e, AnalysisEvent::Stopped { .. })).await;
    let AnalysisEvent::Stopped {
        reason, counters, ..
    } = stopped
    else {
        unreachable!()
    };
    assert_eq!(reason, AnalysisEndReason::Revoked);
    assert!(counters.blocks <= 30);
    // An open segment is closed as a stream end, never left dangling.
    let evs = analysis_events(&h);
    let starts = evs
        .iter()
        .filter(|e| matches!(e, AnalysisEvent::SegmentStarted { .. }))
        .count();
    let ends: Vec<_> = evs
        .iter()
        .filter_map(|e| {
            if let AnalysisEvent::SegmentEnded { reason, .. } = e {
                Some(*reason)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(starts, ends.len());
    assert_eq!(ends.last(), Some(&SegmentCloseReason::StreamEnd));
}

#[tokio::test]
async fn device_loss_ends_analysis_as_invalidated() {
    let mut h = audio_harness().await;
    let (mut reader, id) = running_capture(&mut h).await;
    h.engine
        .start_analysis_with(id, Some(functional()), Box::new(burst()))
        .await
        .unwrap();
    feed(&h, &mut reader, 5).await;
    h.fake.remove_device("mic");
    let stopped = wait_analysis(&mut h, |e| matches!(e, AnalysisEvent::Stopped { .. })).await;
    assert!(
        matches!(
            stopped,
            AnalysisEvent::Stopped {
                reason: AnalysisEndReason::StreamInvalidated,
                ..
            }
        ),
        "{stopped:?}"
    );
}

#[tokio::test]
async fn a_panicking_detector_fails_only_the_analysis() {
    let mut h = audio_harness().await;
    let (mut reader, id) = running_capture(&mut h).await;
    h.engine
        .start_analysis_with(
            id,
            Some(functional()),
            Box::new(ScriptedVad::new(|_| 0.9).panic_at(25)),
        )
        .await
        .unwrap();
    feed(&h, &mut reader, 30).await;
    let failed = wait_analysis(&mut h, |e| matches!(e, AnalysisEvent::Failed { .. })).await;
    let AnalysisEvent::Failed { error, .. } = failed else {
        unreachable!()
    };
    assert_eq!(error.code, "analysis.worker_panicked");
    let stopped = wait_analysis(&mut h, |e| matches!(e, AnalysisEvent::Stopped { .. })).await;
    assert!(matches!(
        stopped,
        AnalysisEvent::Stopped {
            reason: AnalysisEndReason::Failed,
            ..
        }
    ));
    assert_eq!(feed(&h, &mut reader, 20).await, 20, "capture is unaffected");
    let s = h.engine.audio_streams().await.unwrap();
    assert_eq!(s[0].state, AudioStreamState::Running);
    assert_eq!(h.engine.metrics().analysis_failures, 1);
    let st = h.engine.analysis_status(id).await.unwrap();
    assert_eq!(st.ended, Some(AnalysisEndReason::Failed));
    // A new analysis can start on the same stream.
    h.engine
        .start_analysis_with(id, Some(functional()), Box::new(burst()))
        .await
        .unwrap();
}

#[tokio::test]
async fn overload_degrades_and_drops_analysis_copies_but_never_capture() {
    let mut config = audio_config();
    config.analysis.budget.queue_blocks = 8;
    config.analysis.budget.degrade_after = 3;
    let mut h = audio_harness_with(config, FakeDriver::Manual).await;
    let (mut reader, id) = running_capture(&mut h).await;
    // 15 ms of work per 10 ms hop: load 1.5.
    h.engine
        .start_analysis_with(
            id,
            None,
            Box::new(ScriptedVad::new(|_| 0.5).with_delay(Duration::from_millis(15))),
        )
        .await
        .unwrap();
    assert_eq!(
        feed(&h, &mut reader, 120).await,
        120,
        "every block reaches the host"
    );
    let e = wait_analysis(&mut h, |e| {
        matches!(e, AnalysisEvent::DegradedModeChanged { .. })
    })
    .await;
    let AnalysisEvent::DegradedModeChanged {
        from,
        to,
        load_permille,
        ..
    } = e
    else {
        unreachable!()
    };
    assert_eq!((from, to), (DegradedMode::Full, DegradedMode::Reduced));
    assert!(load_permille > 500);
    // Still over budget: the next step sheds half the blocks; activity is
    // suspended (an open segment would close with ResourcePolicy).
    let e = wait_analysis(&mut h, |e| {
        matches!(
            e,
            AnalysisEvent::DegradedModeChanged {
                to: DegradedMode::Shedding,
                ..
            }
        )
    })
    .await;
    assert!(matches!(
        e,
        AnalysisEvent::DegradedModeChanged {
            from: DegradedMode::Reduced,
            ..
        }
    ));
    let st = h.engine.stop_analysis(id).await.unwrap();
    assert_eq!(st.mode, DegradedMode::Shedding);
    assert!(st.counters.shed_blocks > 0 || st.counters.tap_dropped_blocks > 0);
    assert!(st.counters.tap_dropped_blocks > 0, "{:?}", st.counters);
    assert!(st.counters.max_load_permille > 1000);
    let audio = h.engine.audio_streams().await.unwrap();
    assert_eq!(
        audio[0].counters.queue_dropped_blocks, 0,
        "host delivery never dropped"
    );
}

#[tokio::test]
async fn a_stalled_worker_is_detected_and_reported() {
    let mut config = audio_config();
    config.analysis.budget.stall_timeout_ms = 150;
    let mut h = audio_harness_with(config, FakeDriver::Manual).await;
    let (mut reader, id) = running_capture(&mut h).await;
    h.engine
        .start_analysis_with(
            id,
            None,
            Box::new(ScriptedVad::new(|_| 0.1).with_delay(Duration::from_millis(600))),
        )
        .await
        .unwrap();
    feed(&h, &mut reader, 1).await;
    let failed = wait_analysis(&mut h, |e| matches!(e, AnalysisEvent::Failed { .. })).await;
    let AnalysisEvent::Failed { error, .. } = failed else {
        unreachable!()
    };
    assert_eq!(error.code, "analysis.stalled");
    let st = h.engine.analysis_status(id).await.unwrap();
    assert_eq!(
        (st.running, st.ended),
        (false, Some(AnalysisEndReason::Failed))
    );
    assert_eq!(feed(&h, &mut reader, 5).await, 5);
}

#[tokio::test]
async fn shutdown_and_session_close_stop_analyses_with_explicit_reasons() {
    let mut h = audio_harness().await;
    let (mut reader, id) = running_capture(&mut h).await;
    h.engine
        .start_analysis_with(
            id,
            Some(functional()),
            Box::new(ScriptedVad::new(|i| if i >= 10 { 0.95 } else { 0.0 })),
        )
        .await
        .unwrap();
    feed(&h, &mut reader, 40).await;
    wait_analysis(&mut h, |e| {
        matches!(e, AnalysisEvent::SegmentStarted { .. })
    })
    .await;
    let AudioHarness {
        engine, mut events, ..
    } = h;
    let report = engine.shutdown().await.unwrap();
    assert!(report.clean, "{report:?}");
    events.drain_until_closed(WAIT).await;
    let evs: Vec<AnalysisEvent> = events
        .events()
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::Analysis(a) => Some(a.clone()),
            _ => None,
        })
        .collect();
    assert!(evs.iter().any(|e| matches!(
        e,
        AnalysisEvent::Stopped {
            reason: AnalysisEndReason::Shutdown,
            ..
        }
    )));
    assert!(evs.iter().any(|e| matches!(
        e,
        AnalysisEvent::SegmentEnded {
            reason: SegmentCloseReason::ExplicitStop,
            ..
        }
    )));
    assert!(evs.iter().any(|e| matches!(
        e,
        AnalysisEvent::ActivityChanged {
            state: ActivityState::Inactive,
            ..
        }
    )));

    let mut h = audio_harness().await;
    let (mut reader, id) = running_capture(&mut h).await;
    h.engine
        .start_analysis_with(id, Some(functional()), Box::new(burst()))
        .await
        .unwrap();
    feed(&h, &mut reader, 3).await;
    h.engine.close_session(h.session.id).await.unwrap();
    let stopped = wait_analysis(&mut h, |e| matches!(e, AnalysisEvent::Stopped { .. })).await;
    assert!(
        matches!(
            stopped,
            AnalysisEvent::Stopped {
                reason: AnalysisEndReason::StreamClosed,
                ..
            }
        ),
        "{stopped:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_analyses_on_live_streams_are_isolated() {
    let driver = FakeDriver::Realtime {
        period: Duration::from_millis(5),
    };
    let h = audio_harness_with(audio_config(), driver).await;
    h.grant_capture().await;
    let mut readers = Vec::new();
    for k in 0..4u64 {
        let r = h
            .engine
            .open_capture(h.session.id, capture_16k_mono())
            .await
            .unwrap();
        let id = r.info().id;
        h.engine.start_audio(id).await.unwrap();
        // Stream 2 gets a detector that panics; the others must not notice.
        let vad = if k == 2 {
            ScriptedVad::new(|_| 0.9).panic_at(5)
        } else {
            ScriptedVad::new(|i| if i % 100 < 50 { 0.95 } else { 0.0 })
        };
        h.engine
            .start_analysis_with(id, Some(functional()), Box::new(vad))
            .await
            .unwrap();
        readers.push(r);
    }
    let deadline = tokio::time::Instant::now() + Duration::from_millis(1_500);
    let mut received = [0usize; 4];
    while tokio::time::Instant::now() < deadline {
        for (k, r) in readers.iter_mut().enumerate() {
            while let Ok(Some(_)) = r.try_recv() {
                received[k] += 1;
            }
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(
        received.iter().all(|n| *n > 50),
        "all streams keep delivering: {received:?}"
    );
    let ids: Vec<_> = readers.iter().map(|r| r.info().id).collect();
    for (k, id) in ids.iter().enumerate() {
        let st = h.engine.analysis_status(*id).await.unwrap();
        if k == 2 {
            assert_eq!(st.ended, Some(AnalysisEndReason::Failed));
        } else {
            assert!(st.running, "stream {k}: {st:?}");
            assert!(st.counters.blocks > 50);
        }
    }
    let health = h.engine.health().await.unwrap();
    assert_eq!(health.analyses.len(), 4);
    let report = h.engine.shutdown().await.unwrap();
    assert!(report.clean, "{report:?}");
}
