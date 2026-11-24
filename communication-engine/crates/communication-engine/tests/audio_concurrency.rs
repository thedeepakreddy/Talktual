//! Concurrency with real device threads (fake realtime driver): shutdown
//! and drop during active streams, removal between open and start, many
//! concurrent streams, and privacy of audio in logs and events.

mod common;

use std::sync::Arc;
use std::time::Duration;

use ce_testkit::fake_audio::FakeDriver;
use common::*;
use communication_engine::contracts::audio::AudioEvent;
use communication_engine::contracts::event::StopReason;
use communication_engine::prelude::*;

const REALTIME: FakeDriver = FakeDriver::Realtime {
    period: Duration::from_millis(5),
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_during_active_streams_quiesces_callbacks_before_release() {
    let h = audio_harness_with(audio_config(), REALTIME).await;
    h.grant_capture().await;
    let mut reader = h
        .engine
        .open_capture(h.session.id, StreamRequest::capture())
        .await
        .unwrap();
    let mut writer = h
        .engine
        .open_playback(h.session.id, StreamRequest::playback())
        .await
        .unwrap();
    h.engine.start_audio(reader.info().id).await.unwrap();
    h.engine.start_audio(writer.info().id).await.unwrap();
    let ch = usize::from(writer.info().format.delivered.channels);
    writer.write(vec![0.1; 4_800 * ch]).await.unwrap();
    // Real callbacks are flowing.
    let first = recv_block(&mut reader).await;
    let second = recv_block(&mut reader).await;
    assert_eq!(second.meta.frame_index, first.meta.end_frame_index());
    let report = h.engine.shutdown().await.unwrap();
    assert!(report.clean);
    assert_eq!(report.audio_streams_closed, 2);
    for s in h.fake.streams() {
        assert!(s.is_closed() && !s.holds_endpoint());
        assert_eq!(
            s.callbacks_after_close(),
            0,
            "no callback ran after release"
        );
    }
    // The reader drains what was captured, then ends.
    loop {
        match tokio::time::timeout(WAIT, reader.recv()).await.unwrap() {
            Ok(_) => continue,
            Err(e) => {
                assert_eq!(e, ReadError::Closed);
                break;
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropping_the_engine_mid_stream_releases_devices_in_the_background() {
    let mut h = audio_harness_with(audio_config(), REALTIME).await;
    h.grant_capture().await;
    let mut reader = h
        .engine
        .open_capture(h.session.id, StreamRequest::capture())
        .await
        .unwrap();
    h.engine.start_audio(reader.info().id).await.unwrap();
    recv_block(&mut reader).await;
    drop(h.engine);
    assert!(h.events.drain_until_closed(WAIT).await);
    let reason = h.events.events().iter().find_map(|e| match e.payload {
        EventPayload::RuntimeStopping { reason } => Some(reason),
        _ => None,
    });
    assert_eq!(reason, Some(StopReason::HandleDropped));
    assert!(
        h.fake
            .streams()
            .iter()
            .all(|s| s.is_closed() && !s.holds_endpoint())
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn device_removal_between_open_and_start_never_starts_a_dead_stream() {
    let mut h = audio_harness_with(audio_config(), REALTIME).await;
    h.grant_capture().await;
    let reader = h
        .engine
        .open_capture(h.session.id, StreamRequest::capture())
        .await
        .unwrap();
    let id = reader.info().id;
    h.fake.remove_device("mic");
    h.wait_audio(|e| matches!(e, AudioEvent::StreamInvalidated { .. }))
        .await;
    let err = h.engine.start_audio(id).await.unwrap_err();
    assert_eq!(err.code(), "audio.stream_not_startable");
    assert!(!h.fake.last_stream().unwrap().is_running());
    h.engine.close_audio(id).await.unwrap();
    h.engine.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn many_concurrent_streams_and_callers_stay_consistent() {
    let mut config = audio_config();
    config.audio.max_streams = 16;
    let h = audio_harness_with(config, REALTIME).await;
    let engine = Arc::new(h.engine);
    let mut tasks = Vec::new();
    for _ in 0..4 {
        let e = engine.clone();
        tasks.push(tokio::spawn(async move {
            let s = e.create_session(SessionConfig::default()).await.unwrap();
            let at = WallTimeNow::now();
            e.update_consent(s.id, ConsentPurpose::Capture, ConsentState::Granted { at })
                .await
                .unwrap();
            let mut reader = e
                .open_capture(s.id, StreamRequest::capture())
                .await
                .unwrap();
            let writer = e
                .open_playback(s.id, StreamRequest::playback())
                .await
                .unwrap();
            for _ in 0..3 {
                e.start_audio(reader.info().id).await.unwrap();
                e.start_audio(writer.info().id).await.unwrap();
                let b = recv_block(&mut reader).await;
                assert_eq!(b.meta.session_id, s.id);
                e.stop_audio(reader.info().id).await.unwrap();
                e.stop_audio(writer.info().id).await.unwrap();
            }
            e.close_session(s.id).await.unwrap();
        }));
    }
    for t in tasks {
        t.await.unwrap();
    }
    assert!(engine.audio_streams().await.unwrap().is_empty());
    assert_eq!(engine.metrics().audio_streams_opened, 8);
    let engine = Arc::into_inner(engine).unwrap();
    assert!(engine.shutdown().await.unwrap().clean);
    assert!(h.fake.streams().iter().all(|s| s.is_closed()));
}

/// Wall time for consent grants in tests without a harness clock.
struct WallTimeNow;
impl WallTimeNow {
    fn now() -> communication_engine::contracts::time::WallTime {
        use communication_engine::contracts::time::Clock;
        communication_engine::contracts::time::SystemClock::new().wall()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn audio_never_appears_in_logs_or_events() {
    let sink = Arc::new(ce_testkit::RecordingSink::default());
    let fake = ce_testkit::fake_audio::FakeAudioBackend::new(REALTIME);
    let engine = Engine::builder()
        .config(audio_config())
        .observability(sink.clone())
        .audio_backend(Arc::new(fake.clone()))
        .build()
        .unwrap();
    let mut events = ce_testkit::EventRecorder::new(engine.subscribe().unwrap());
    engine.start().await.unwrap();
    let s = engine
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    engine
        .update_consent(
            s.id,
            ConsentPurpose::Capture,
            ConsentState::Granted {
                at: WallTimeNow::now(),
            },
        )
        .await
        .unwrap();
    let mut reader = engine
        .open_capture(s.id, StreamRequest::capture())
        .await
        .unwrap();
    engine.start_audio(reader.info().id).await.unwrap();
    recv_block(&mut reader).await;
    engine.shutdown().await.unwrap();
    events.drain_until_closed(WAIT).await;
    let logs = sink.rendered();
    assert!(!logs.contains("Fake mic"), "device names are personal data");
    for e in events.events() {
        if let EventPayload::Audio(_) = e.payload {
            assert_eq!(e.privacy, DataClass::Operational);
        }
        let json = serde_json::to_string(e.as_ref()).unwrap();
        assert!(
            !json.contains("Fake mic") && !json.contains("\"samples\""),
            "{json}"
        );
    }
    assert!(logs.contains("audio.stream_state_changed"));
}
