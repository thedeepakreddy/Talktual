//! The real `cpal` adapter, end to end through the facade, on ALSA's
//! virtual `null` PCM (capture produces zeros, playback discards).
//!
//! This exercises real cpal/ALSA code paths — enumeration, negotiation,
//! stream build, callbacks on cpal's thread, start/stop, drop — but **no
//! audio hardware**. It is skipped (with a message) where the ALSA null
//! device does not exist.
#![cfg(all(feature = "cpal", target_os = "linux"))]
// The test reports what it measured on the virtual device.
#![allow(clippy::print_stderr)]

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;
use communication_engine::audio::cpal_backend::CpalBackend;
use communication_engine::prelude::*;

const NULL_DEVICE_NAME: &str = "Discard all samples";

async fn engine() -> (Engine, SessionInfo) {
    let engine = Engine::builder()
        .config(audio_config())
        .audio_backend(Arc::new(CpalBackend::default_host()))
        .build()
        .unwrap();
    engine.start().await.unwrap();
    let s = engine
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    let at = {
        use communication_engine::contracts::time::Clock;
        communication_engine::contracts::time::SystemClock::new().wall()
    };
    engine
        .update_consent(s.id, ConsentPurpose::Capture, ConsentState::Granted { at })
        .await
        .unwrap();
    (engine, s)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cpal_alsa_null_device_capture_and_playback_end_to_end() {
    let (engine, s) = engine().await;
    let catalog = engine.audio_devices().await.unwrap();
    assert!(
        catalog.backend.name.starts_with("cpal/"),
        "{}",
        catalog.backend.name
    );
    let Some(null) = catalog
        .devices
        .iter()
        .find(|d| d.name.starts_with(NULL_DEVICE_NAME))
    else {
        eprintln!("SKIP: ALSA null device not present on this host");
        engine.shutdown().await.unwrap();
        return;
    };
    assert!(null.supports_input && null.supports_output);
    // The null PCM accepts any rate; CE clamps the reported range to its limits.
    assert!(
        null.input_formats
            .iter()
            .all(|r| r.max_sample_rate <= 768_000)
    );

    let request = StreamRequest {
        device: DeviceSelector::Device(null.id),
        format: FormatPreference {
            sample_rate: Some(16_000),
            channels: Some(1),
            sample_format: Some(SampleFormat::I16),
            buffer_frames: Some(160),
        },
        ..StreamRequest::capture()
    };
    let mut reader = engine.open_capture(s.id, request).await.unwrap();
    let f = reader.info().format.clone();
    assert_eq!(
        (
            f.device.sample_rate,
            f.device.channels,
            f.device.sample_format
        ),
        (16_000, 1, SampleFormat::I16)
    );
    let id = reader.info().id;
    let started = Instant::now();
    engine.start_audio(id).await.unwrap();
    // ALSA's null PCM has no hardware clock: it produces frames as fast as
    // they are read, which overruns CE's ring. Every gap must be flagged
    // with its exact size, and the flagged total must match the counter.
    let mut frames = 0u64;
    let mut flagged_lost = 0u64;
    let mut expected_index = 0;
    while frames < 16_000 / 2 {
        let b = tokio::time::timeout(Duration::from_secs(5), reader.recv())
            .await
            .expect("block in time")
            .expect("block");
        if b.meta.frame_index != expected_index {
            let d = b
                .meta
                .discontinuity
                .expect("every jump in the sample clock is flagged");
            assert_eq!(
                d.lost_frames,
                Some(b.meta.frame_index - expected_index),
                "flagged size is exact"
            );
            flagged_lost += b.meta.frame_index - expected_index;
        }
        expected_index = b.meta.end_frame_index();
        assert!(
            b.samples.iter().all(|s| *s == 0.0),
            "the null PCM captures silence"
        );
        frames += u64::from(b.meta.frames);
    }
    let elapsed = started.elapsed();
    engine.stop_audio(id).await.unwrap();
    let status = engine.close_audio(id).await.unwrap();
    assert!(status.counters.callbacks > 0);
    assert!(
        flagged_lost <= status.counters.ring_overflow_frames + status.counters.queue_dropped_frames,
        "delivered gaps never exceed counted loss"
    );
    eprintln!(
        "alsa-null capture: {frames} frames delivered in {elapsed:?}, callbacks={}, actual_buffer={:?}, \
         ring_overflow_frames={}, flagged_lost={flagged_lost}, discontinuities={}",
        status.counters.callbacks,
        f.actual_buffer_frames,
        status.counters.ring_overflow_frames,
        status.counters.discontinuities
    );

    let mut writer = engine
        .open_playback(
            s.id,
            StreamRequest {
                device: DeviceSelector::Device(null.id),
                ..StreamRequest::playback()
            },
        )
        .await
        .unwrap();
    let ch = usize::from(writer.info().format.delivered.channels);
    let rate = writer.info().format.device.sample_rate as usize;
    engine.start_audio(writer.info().id).await.unwrap();
    writer.write(vec![0.25; rate / 10 * ch]).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let status = engine.close_audio(writer.info().id).await.unwrap();
    assert!(
        status.counters.frames >= (rate / 10) as u64,
        "the device consumed the written audio: {status:?}"
    );
    assert!(engine.shutdown().await.unwrap().clean);
}

#[tokio::test]
async fn an_unusable_default_device_is_a_typed_error_not_a_crash() {
    let (engine, s) = engine().await;
    let catalog = engine.audio_devices().await.unwrap();
    if catalog.default_input.is_some() {
        eprintln!("SKIP: this host has a usable default input");
        engine.shutdown().await.unwrap();
        return;
    }
    // In this container ALSA's `default` PCM has no card behind it, so cpal
    // cannot describe it and CE leaves it out of the catalog.
    let err = engine
        .open_capture(s.id, StreamRequest::capture())
        .await
        .unwrap_err();
    assert_eq!(
        (err.category(), err.code()),
        (ErrorCategory::Device, "audio.no_default_device")
    );
    engine.shutdown().await.unwrap();
}
