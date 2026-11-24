//! Audio stream lifecycle, format negotiation, framing and timestamps,
//! through the public facade with the fake backend.

mod common;

use ce_testkit::fake_audio::expected_sample;
use common::*;
use communication_engine::contracts::audio::{
    AudioEvent, DiscontinuityKind, FormatAdjustment, TerminalReason,
};
use communication_engine::prelude::*;

fn capture_16k_mono_f32() -> StreamRequest {
    StreamRequest {
        format: FormatPreference {
            sample_rate: Some(16_000),
            channels: Some(1),
            sample_format: Some(SampleFormat::F32),
            buffer_frames: None,
        },
        ..StreamRequest::capture()
    }
}

#[tokio::test]
async fn capture_lifecycle_reports_resolved_format_and_delivers_ordered_timestamped_blocks() {
    let mut h = audio_harness().await;
    h.grant_capture().await;
    let mut reader = h
        .engine
        .open_capture(h.session.id, capture_16k_mono_f32())
        .await
        .unwrap();
    let info = reader.info().clone();
    // The fake offers only i16: the resolved format says so exactly.
    assert_eq!(info.format.device.sample_rate, 16_000);
    assert_eq!(info.format.device.channels, 1);
    assert_eq!(info.format.device.sample_format, SampleFormat::I16);
    assert_eq!(
        info.format.adjustments,
        vec![FormatAdjustment::SampleFormat {
            requested: SampleFormat::F32,
            resolved: SampleFormat::I16
        }]
    );
    assert_eq!(info.format.delivered.sample_format, SampleFormat::F32);
    assert_eq!(info.block_frames, 160, "10 ms blocks at 16 kHz");
    assert_eq!(
        (info.session_id, info.correlation_id),
        (h.session.id, h.session.correlation_id)
    );

    let status = h.engine.start_audio(info.id).await.unwrap();
    assert_eq!(status.state, AudioStreamState::Running);
    for _ in 0..5 {
        h.fake.tick(160);
    }
    let mut blocks = Vec::new();
    for _ in 0..5 {
        blocks.push(recv_block(&mut reader).await);
    }
    for (i, b) in blocks.iter().enumerate() {
        let m = &b.meta;
        assert_eq!(
            (m.sequence, m.frame_index, m.frames),
            (i as u64, i as u64 * 160, 160)
        );
        assert_eq!(
            (m.stream_id, m.session_id, m.correlation_id),
            (info.id, h.session.id, h.session.correlation_id)
        );
        assert_eq!(
            (m.sample_rate, m.channels, m.sample_format),
            (16_000, 1, SampleFormat::F32)
        );
        assert_eq!(m.discontinuity, None);
        assert!(m.device_time.is_some(), "fake reports a device clock");
        for (k, s) in b.samples.iter().enumerate() {
            let expected = expected_sample(m.frame_index + k as u64, 0);
            assert!(
                (s - expected).abs() <= 1.0 / 32_768.0,
                "frame {} {s} vs {expected}",
                m.frame_index + k as u64
            );
        }
    }
    // Timestamps follow the sample clock exactly: 10 ms apart.
    for w in blocks.windows(2) {
        assert_eq!(
            w[1].meta.timestamp.as_nanos() - w[0].meta.timestamp.as_nanos(),
            10_000_000
        );
    }

    // Stop flushes a partial block; restart flags the real-time gap.
    h.fake.tick(100);
    h.engine.stop_audio(info.id).await.unwrap();
    let partial = recv_block(&mut reader).await;
    assert_eq!((partial.meta.frames, partial.meta.frame_index), (100, 800));
    h.fake.tick(160); // stopped: no callback runs
    h.engine.start_audio(info.id).await.unwrap();
    h.fake.tick(160);
    let restarted = recv_block(&mut reader).await;
    assert_eq!(
        restarted.meta.frame_index, 900,
        "the sample clock continues across restarts"
    );
    assert_eq!(
        restarted.meta.discontinuity.unwrap().kind,
        DiscontinuityKind::StreamRestart
    );

    let fake_stream = h.fake.last_stream().unwrap();
    let closed = h.engine.close_audio(info.id).await.unwrap();
    assert_eq!(closed.state, AudioStreamState::Closed);
    assert_eq!(closed.terminal, Some(TerminalReason::Closed));
    assert_eq!(closed.counters.blocks, 7);
    // Callback quiescence: the backend released the endpoint before close returned.
    assert!(fake_stream.is_closed() && !fake_stream.holds_endpoint());
    fake_stream.callback(160);
    assert_eq!(
        fake_stream.callbacks_after_close(),
        1,
        "the attempt was refused, not delivered"
    );
    assert_eq!(reader.recv().await.unwrap_err(), ReadError::Closed);
    assert!(h.engine.audio_streams().await.unwrap().is_empty());

    // Lifecycle events, in order, all carrying the session correlation id.
    h.engine.execute(Command::Shutdown).await.unwrap();
    h.events.drain_until_closed(WAIT).await;
    let states: Vec<_> = h
        .audio_events()
        .into_iter()
        .filter_map(|e| match e {
            AudioEvent::StreamStateChanged { to, .. } => Some(to),
            _ => None,
        })
        .collect();
    use AudioStreamState::*;
    assert_eq!(
        states,
        vec![Configured, Open, Running, Stopped, Running, Closed]
    );
    for e in h.events.events() {
        if let EventPayload::Audio(AudioEvent::StreamStateChanged { .. }) = &e.payload {
            assert_eq!(e.correlation_id, Some(h.session.correlation_id));
            assert_eq!(e.session_id, Some(h.session.id));
        }
    }
    assert!(
        h.audio_events()
            .iter()
            .any(|e| matches!(e, AudioEvent::StreamFormatResolved { .. }))
    );
}

#[tokio::test]
async fn illegal_transitions_are_rejected_without_side_effects() {
    let h = audio_harness().await;
    h.grant_capture().await;
    let reader = h
        .engine
        .open_capture(h.session.id, StreamRequest::capture())
        .await
        .unwrap();
    let id = reader.info().id;
    let err = h.engine.stop_audio(id).await.unwrap_err();
    assert_eq!(
        (err.category(), err.code()),
        (ErrorCategory::Lifecycle, "audio.stream_not_running")
    );
    h.engine.start_audio(id).await.unwrap();
    assert_eq!(
        h.engine.start_audio(id).await.unwrap_err().code(),
        "audio.stream_not_startable"
    );
    h.engine.stop_audio(id).await.unwrap();
    assert_eq!(
        h.engine.stop_audio(id).await.unwrap_err().code(),
        "audio.stream_not_running",
        "double stop"
    );
    h.engine.close_audio(id).await.unwrap();
    assert_eq!(
        h.engine.close_audio(id).await.unwrap_err().code(),
        "audio.stream_not_found",
        "double close / stale handle"
    );
    assert_eq!(
        h.engine.start_audio(id).await.unwrap_err().code(),
        "audio.stream_not_found"
    );
    // Requests are validated before anything opens.
    let mut bad = StreamRequest::playback();
    bad.channel_mode = ChannelMode::DownmixToMono;
    assert_eq!(
        h.engine
            .open_playback(h.session.id, bad)
            .await
            .unwrap_err()
            .code(),
        "audio.invalid_channel_mode"
    );
    assert_eq!(
        h.engine
            .open_capture(h.session.id, StreamRequest::playback())
            .await
            .unwrap_err()
            .code(),
        "audio.direction_mismatch"
    );
    assert_eq!(h.fake.streams().len(), 1);
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn repeated_start_stop_cycles_keep_a_consistent_sample_clock() {
    let h = audio_harness().await;
    h.grant_capture().await;
    let mut reader = h
        .engine
        .open_capture(h.session.id, StreamRequest::capture())
        .await
        .unwrap();
    let id = reader.info().id;
    let block = reader.info().block_frames as usize;
    let mut expected_index = 0;
    for cycle in 0..20 {
        h.engine.start_audio(id).await.unwrap();
        h.fake.tick(block);
        let b = recv_block(&mut reader).await;
        assert_eq!(b.meta.frame_index, expected_index);
        assert_eq!(b.meta.sequence, cycle);
        if cycle > 0 {
            assert_eq!(
                b.meta.discontinuity.unwrap().kind,
                DiscontinuityKind::StreamRestart
            );
        }
        expected_index += block as u64;
        h.engine.stop_audio(id).await.unwrap();
    }
    h.engine.close_audio(id).await.unwrap();
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn downmix_averages_channels_and_native_mode_keeps_them() {
    let h = audio_harness().await;
    h.grant_capture().await;
    let stereo = FormatPreference {
        channels: Some(2),
        sample_rate: Some(48_000),
        ..Default::default()
    };
    let mut mono = h
        .engine
        .open_capture(
            h.session.id,
            StreamRequest {
                format: stereo,
                channel_mode: ChannelMode::DownmixToMono,
                ..StreamRequest::capture()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        (
            mono.info().format.device.channels,
            mono.info().format.delivered.channels
        ),
        (2, 1)
    );
    let mut native = h
        .engine
        .open_capture(
            h.session.id,
            StreamRequest {
                format: stereo,
                ..StreamRequest::capture()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        native.info().format.delivered.layout,
        communication_engine::contracts::audio::ChannelLayout::Stereo
    );
    h.engine.start_audio(mono.info().id).await.unwrap();
    h.engine.start_audio(native.info().id).await.unwrap();
    h.fake.tick(480);
    let m = recv_block(&mut mono).await;
    let n = recv_block(&mut native).await;
    assert_eq!((m.samples.len(), n.samples.len()), (480, 960));
    // The fake's right channel is the negated left one, so the mean is 0.
    assert!(m.samples.iter().all(|s| s.abs() <= 1.0 / 32_768.0));
    for (k, frame) in n.samples.chunks_exact(2).enumerate() {
        assert!((frame[0] - expected_sample(k as u64, 0)).abs() <= 1.0 / 32_768.0);
        assert!((frame[1] - expected_sample(k as u64, 1)).abs() <= 1.0 / 32_768.0);
    }
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn actual_buffer_size_is_reported_when_it_differs_from_the_request() {
    let h = audio_harness().await;
    h.grant_capture().await;
    h.fake.report_buffer_frames(Some(512));
    let req = StreamRequest {
        format: FormatPreference {
            buffer_frames: Some(128),
            ..Default::default()
        },
        ..StreamRequest::capture()
    };
    let reader = h.engine.open_capture(h.session.id, req).await.unwrap();
    let f = &reader.info().format;
    assert_eq!(
        (f.requested_buffer_frames, f.actual_buffer_frames),
        (Some(128), Some(512))
    );
    assert!(f.adjustments.contains(&FormatAdjustment::BufferFrames {
        requested: 128,
        resolved: Some(512)
    }));
    // The status (from the runtime) carries the refined format too.
    let status = h.engine.audio_streams().await.unwrap();
    assert_eq!(status[0].info.format.actual_buffer_frames, Some(512));
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn session_close_and_shutdown_release_every_stream() {
    let h = audio_harness().await;
    h.grant_capture().await;
    let a = h
        .engine
        .open_capture(h.session.id, StreamRequest::capture())
        .await
        .unwrap();
    h.engine.start_audio(a.info().id).await.unwrap();
    h.engine
        .open_playback(h.session.id, StreamRequest::playback())
        .await
        .unwrap();
    h.engine.close_session(h.session.id).await.unwrap();
    assert!(h.engine.audio_streams().await.unwrap().is_empty());
    assert!(h.fake.streams().iter().all(|s| s.is_closed()));

    let s2 = h
        .engine
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    h.engine
        .open_playback(s2.id, StreamRequest::playback())
        .await
        .unwrap();
    let report = h.engine.shutdown().await.unwrap();
    assert_eq!(report.audio_streams_closed, 1);
    assert!(report.clean);
    assert!(
        h.fake
            .streams()
            .iter()
            .all(|s| s.is_closed() && !s.holds_endpoint())
    );
}

#[tokio::test]
async fn the_stream_limit_is_enforced_and_no_backend_is_a_typed_error() {
    let mut config = audio_config();
    config.audio.max_streams = 2;
    let h = audio_harness_with(config, ce_testkit::fake_audio::FakeDriver::Manual).await;
    h.engine
        .open_playback(h.session.id, StreamRequest::playback())
        .await
        .unwrap();
    h.engine
        .open_playback(h.session.id, StreamRequest::playback())
        .await
        .unwrap();
    let err = h
        .engine
        .open_playback(h.session.id, StreamRequest::playback())
        .await
        .unwrap_err();
    assert_eq!(
        (err.code(), err.is_retryable()),
        ("audio.stream_limit", true)
    );
    h.engine.shutdown().await.unwrap();

    let plain = harness();
    plain.engine.start().await.unwrap();
    let s = plain
        .engine
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    assert_eq!(
        plain.engine.audio_devices().await.unwrap_err().code(),
        "audio.no_backend"
    );
    assert_eq!(
        plain
            .engine
            .open_playback(s.id, StreamRequest::playback())
            .await
            .unwrap_err()
            .code(),
        "audio.no_backend"
    );
    plain.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn streams_need_a_running_runtime_and_an_active_session() {
    let h = audio_harness().await;
    let unknown = SessionId::from_uuid(uuid::Uuid::from_u128(999));
    assert_eq!(
        h.engine
            .open_playback(unknown, StreamRequest::playback())
            .await
            .unwrap_err()
            .code(),
        "session.not_found"
    );
    h.engine.execute(Command::Shutdown).await.unwrap();
    assert_eq!(
        h.engine
            .open_playback(h.session.id, StreamRequest::playback())
            .await
            .unwrap_err()
            .category(),
        ErrorCategory::Shutdown
    );
}
