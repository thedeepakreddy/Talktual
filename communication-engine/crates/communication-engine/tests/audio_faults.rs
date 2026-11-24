//! Device invalidation, backend failures, timeouts, hangs, panics,
//! reroutes and cleanup.

mod common;

use std::time::Duration;

use ce_audio::StreamFault;
use ce_testkit::fake_audio::{FakeDriver, FakeError};
use common::*;
use communication_engine::contracts::audio::{AudioEvent, InvalidationReason, TerminalReason};
use communication_engine::prelude::*;

#[tokio::test]
async fn device_loss_invalidates_without_silent_restart_and_keeps_captured_data() {
    let mut h = audio_harness().await;
    h.grant_capture().await;
    let mut reader = h
        .engine
        .open_capture(h.session.id, StreamRequest::capture())
        .await
        .unwrap();
    let id = reader.info().id;
    h.engine.start_audio(id).await.unwrap();
    h.fake.tick(480);
    let before = recv_block(&mut reader).await;
    h.fake.tick(480);
    h.fake.remove_device("mic");
    // The backend faults the stream *and* notifies a device change; whichever
    // the core sees first names the reason. Both are correct.
    let e = h
        .wait_audio(|e| matches!(e, AudioEvent::StreamInvalidated { .. }))
        .await;
    let AudioEvent::StreamInvalidated { stream, reason } = e else {
        unreachable!()
    };
    assert_eq!(stream, id);
    assert!(matches!(
        reason,
        InvalidationReason::DeviceDisconnected | InvalidationReason::DeviceRemoved
    ));
    // Audio captured before the loss is still delivered, then the reason.
    let after = recv_block(&mut reader).await;
    assert_eq!(after.meta.sequence, before.meta.sequence + 1);
    assert_eq!(
        tokio::time::timeout(WAIT, reader.recv())
            .await
            .unwrap()
            .unwrap_err(),
        ReadError::Invalidated
    );
    let status = &h.engine.audio_streams().await.unwrap()[0];
    assert_eq!(status.state, AudioStreamState::Invalidated);
    assert_eq!(status.terminal, Some(TerminalReason::Invalidated(reason)));
    // No silent restart: starting an invalidated stream is refused.
    assert_eq!(
        h.engine.start_audio(id).await.unwrap_err().code(),
        "audio.stream_not_startable"
    );
    let health = h.engine.health().await.unwrap();
    assert_eq!(health.overall.as_str(), "degraded");
    assert_eq!(health.metrics.audio_streams_invalidated, 1);
    let closed = h.engine.close_audio(id).await.unwrap();
    assert_eq!(closed.state, AudioStreamState::Closed);
    assert!(h.fake.last_stream().unwrap().is_closed());
    assert_eq!(
        h.engine.health().await.unwrap().overall,
        HealthStatus::Healthy
    );
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_device_missing_from_re_enumeration_invalidates_its_streams() {
    let mut h = audio_harness().await;
    let writer = h
        .engine
        .open_playback(h.session.id, StreamRequest::playback())
        .await
        .unwrap();
    // A backend that loses a device without telling the stream: the catalog
    // diff catches it.
    h.fake.set_default_output(None);
    let mut d = ce_testkit::fake_audio::device("speaker", false, true);
    d.supports_output = false;
    d.output_formats.clear();
    h.fake.add_device(d);
    let e = h
        .wait_audio(|e| matches!(e, AudioEvent::StreamInvalidated { .. }))
        .await;
    assert_eq!(
        e,
        AudioEvent::StreamInvalidated {
            stream: writer.info().id,
            reason: InvalidationReason::DeviceRemoved
        }
    );
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn open_failures_clean_up_and_leave_nothing_open() {
    let h = audio_harness().await;
    h.fake.fail_open(Some(FakeError::device("fake.busy")));
    let err = h
        .engine
        .open_playback(h.session.id, StreamRequest::playback())
        .await
        .unwrap_err();
    assert_eq!(err.code(), "fake.busy");
    assert!(h.engine.audio_streams().await.unwrap().is_empty());
    assert_eq!(h.engine.metrics().audio_streams_failed, 1);
    h.fake.fail_open(None);
    h.engine
        .open_playback(h.session.id, StreamRequest::playback())
        .await
        .unwrap();
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_panic_in_the_backend_is_contained_and_reported() {
    // The worker's reply is dropped only after the panic hook has run
    // (with RUST_BACKTRACE set it symbolizes a backtrace first). Keep the
    // open deadline far from that so this test checks panic containment,
    // not timing; timeout classification has its own test below.
    let mut config = audio_config();
    config.audio.open_timeout_ms = 10_000;
    let mut h = audio_harness_with(config, FakeDriver::Manual).await;
    h.fake.panic_on_open();
    let err = h
        .engine
        .open_playback(h.session.id, StreamRequest::playback())
        .await
        .unwrap_err();
    assert_eq!(err.code(), "audio.worker_panicked");
    h.wait_audio(|e| matches!(e, AudioEvent::StreamFailed { terminal: true, .. }))
        .await;
    assert!(h.engine.audio_streams().await.unwrap().is_empty());
    // The engine keeps working.
    h.engine
        .open_playback(h.session.id, StreamRequest::playback())
        .await
        .unwrap();
    assert!(h.engine.shutdown().await.unwrap().clean);
}

#[tokio::test]
async fn open_timeouts_fail_and_the_late_worker_is_reaped() {
    let mut config = audio_config();
    config.audio.open_timeout_ms = 100;
    config.audio.operation_timeout_ms = 100;
    let h = audio_harness_with(config, FakeDriver::Manual).await;
    h.fake.delay_open(Duration::from_millis(400));
    let err = h
        .engine
        .open_playback(h.session.id, StreamRequest::playback())
        .await
        .unwrap_err();
    assert_eq!(
        (err.category(), err.code()),
        (ErrorCategory::Timeout, "audio.open_timeout")
    );
    assert!(h.engine.audio_streams().await.unwrap().is_empty());
    h.fake.delay_open(Duration::ZERO);
    // Shutdown waits (bounded) for the late worker, which then releases the
    // device it eventually opened.
    let report = h.engine.shutdown().await.unwrap();
    assert!(report.clean, "{report:?}");
    assert!(h.fake.streams().iter().all(|s| s.is_closed()));
}

#[tokio::test]
async fn start_failures_are_recoverable_and_stop_failures_fail_safe() {
    let mut h = audio_harness().await;
    let writer = h
        .engine
        .open_playback(h.session.id, StreamRequest::playback())
        .await
        .unwrap();
    let id = writer.info().id;
    h.fake
        .fail_next_start(FakeError::device("fake.device_busy"));
    assert_eq!(
        h.engine.start_audio(id).await.unwrap_err().code(),
        "fake.device_busy"
    );
    h.wait_audio(|e| {
        matches!(
            e,
            AudioEvent::StreamFailed {
                terminal: false,
                ..
            }
        )
    })
    .await;
    assert_eq!(
        h.engine.audio_streams().await.unwrap()[0].state,
        AudioStreamState::Open
    );
    h.engine.start_audio(id).await.unwrap();
    h.fake.fail_next_stop(FakeError::device("fake.stop_failed"));
    assert_eq!(
        h.engine.stop_audio(id).await.unwrap_err().code(),
        "fake.stop_failed"
    );
    assert_eq!(
        h.engine.audio_streams().await.unwrap()[0].state,
        AudioStreamState::Failed
    );
    let closed = h.engine.close_audio(id).await.unwrap();
    assert_eq!(closed.state, AudioStreamState::Closed);
    assert!(matches!(closed.terminal, Some(TerminalReason::Failed(_))));
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_hung_backend_is_bounded_and_reported_not_clean() {
    let mut config = audio_config();
    config.audio.operation_timeout_ms = 100;
    config.runtime.shutdown_timeout_ms = 500;
    let h = audio_harness_with(config, FakeDriver::Manual).await;
    let writer = h
        .engine
        .open_playback(h.session.id, StreamRequest::playback())
        .await
        .unwrap();
    h.fake.hang_next_start();
    let err = h.engine.start_audio(writer.info().id).await.unwrap_err();
    assert_eq!(err.code(), "audio.operation_timeout");
    let started = std::time::Instant::now();
    let report = h.engine.shutdown().await.unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "shutdown is bounded"
    );
    assert!(!report.clean, "a stuck device worker is reported");
}

#[tokio::test]
async fn backend_errors_fail_the_stream_and_xruns_are_counted() {
    let mut h = audio_harness().await;
    h.grant_capture().await;
    let reader = h
        .engine
        .open_capture(h.session.id, StreamRequest::capture())
        .await
        .unwrap();
    h.engine.start_audio(reader.info().id).await.unwrap();
    let fs = h.fake.last_stream().unwrap();
    fs.xrun();
    fs.xrun();
    h.fake.tick(480);
    tokio::time::timeout(WAIT, async {
        loop {
            let s = h.engine.audio_streams().await.unwrap();
            if s[0].counters.backend_xruns == 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    fs.fault(StreamFault::BackendError);
    h.wait_audio(|e| matches!(e, AudioEvent::StreamFailed { terminal: true, .. }))
        .await;
    assert_eq!(
        h.engine.audio_streams().await.unwrap()[0].state,
        AudioStreamState::Failed
    );
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn reroutes_and_default_divergence_are_reported_without_moving_the_stream() {
    let mut h = audio_harness().await;
    let writer = h
        .engine
        .open_playback(h.session.id, StreamRequest::playback())
        .await
        .unwrap();
    let id = writer.info().id;
    let bound = writer.info().device;
    h.engine.start_audio(id).await.unwrap();
    h.fake.last_stream().unwrap().reroute();
    h.wait_audio(|e| *e == AudioEvent::StreamRerouted { stream: id })
        .await;
    h.fake
        .add_device(ce_testkit::fake_audio::device("hdmi", false, true));
    h.fake.set_default_output(Some("hdmi"));
    let e = h
        .wait_audio(|e| matches!(e, AudioEvent::StreamDefaultDeviceDiverged { .. }))
        .await;
    let AudioEvent::StreamDefaultDeviceDiverged {
        stream,
        bound_to,
        new_default,
    } = e
    else {
        unreachable!()
    };
    assert_eq!((stream, bound_to), (id, bound));
    assert!(new_default.is_some() && new_default != Some(bound));
    assert_eq!(
        h.engine.audio_streams().await.unwrap()[0].state,
        AudioStreamState::Running
    );
    h.engine.shutdown().await.unwrap();
}
