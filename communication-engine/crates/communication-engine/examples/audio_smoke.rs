//! Audio smoke example (Phase 2): the public audio API end to end, on the
//! test kit's fake backend with a real-time driver thread.
//!
//! discover devices → consent → open capture (16 kHz mono requested) →
//! start → read timestamped blocks → play them back → stop → revoke →
//! shutdown. Exits non-zero on any failed expectation (CI gate).
//! Run: `cargo run -p communication-engine --example audio_smoke`

#![allow(clippy::print_stdout)]

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use ce_testkit::EventRecorder;
use ce_testkit::fake_audio::{FakeAudioBackend, FakeDriver};
use communication_engine::contracts::audio::AudioEvent;
use communication_engine::contracts::time::{Clock, SystemClock};
use communication_engine::prelude::*;

fn check(ok: bool, what: &str) -> Result<(), String> {
    if ok {
        println!("  ok   {what}");
        Ok(())
    } else {
        Err(format!("FAILED: {what}"))
    }
}

async fn run() -> Result<(), String> {
    let err = |e: CeError| e.to_string();
    let fake = FakeAudioBackend::new(FakeDriver::Realtime {
        period: Duration::from_millis(5),
    });
    let engine = Engine::builder()
        .audio_backend(Arc::new(fake.clone()))
        .build()
        .map_err(err)?;
    let mut events = EventRecorder::new(engine.subscribe().map_err(err)?);
    engine.start().await.map_err(err)?;

    let catalog = engine.audio_devices().await.map_err(err)?;
    println!(
        "backend {} {}",
        catalog.backend.name, catalog.backend.version
    );
    for d in &catalog.devices {
        println!(
            "  device {} in={} out={} formats={}",
            d.id,
            d.supports_input,
            d.supports_output,
            d.input_formats.len() + d.output_formats.len()
        );
    }
    check(
        catalog.default_input.is_some() && catalog.default_output.is_some(),
        "default devices discovered",
    )?;

    let session = engine
        .create_session(SessionConfig::default())
        .await
        .map_err(err)?;
    let denied = engine
        .open_capture(session.id, StreamRequest::capture())
        .await;
    check(
        denied
            .as_ref()
            .is_err_and(|e| e.code() == "audio.capture_unauthorized"),
        "capture denied without consent",
    )?;
    engine
        .update_consent(
            session.id,
            ConsentPurpose::Capture,
            ConsentState::Granted {
                at: SystemClock::new().wall(),
            },
        )
        .await
        .map_err(err)?;

    let request = StreamRequest {
        format: FormatPreference {
            sample_rate: Some(16_000),
            channels: Some(1),
            ..Default::default()
        },
        ..StreamRequest::capture()
    };
    let mut reader = engine
        .open_capture(session.id, request)
        .await
        .map_err(err)?;
    let info = reader.info().clone();
    println!(
        "  capture {} device {} resolved {} Hz x{} {} (exact={}) block={} frames",
        info.id,
        info.device,
        info.format.device.sample_rate,
        info.format.device.channels,
        info.format.device.sample_format,
        info.format.is_exact(),
        info.block_frames
    );
    let mut writer = engine
        .open_playback(session.id, StreamRequest::playback())
        .await
        .map_err(err)?;
    engine.start_audio(info.id).await.map_err(err)?;
    engine.start_audio(writer.info().id).await.map_err(err)?;

    let mut blocks = Vec::new();
    for _ in 0..20 {
        let b = tokio::time::timeout(Duration::from_secs(5), reader.recv())
            .await
            .map_err(|_| "timed out waiting for audio".to_owned())?
            .map_err(|e| format!("read: {e:?}"))?;
        blocks.push(b);
    }
    check(
        blocks
            .windows(2)
            .all(|w| w[1].meta.sequence == w[0].meta.sequence + 1),
        "block sequence is gap-free",
    )?;
    check(
        blocks
            .windows(2)
            .all(|w| w[1].meta.frame_index == w[0].meta.end_frame_index()),
        "sample clock is contiguous",
    )?;
    check(
        blocks
            .windows(2)
            .all(|w| w[1].meta.timestamp > w[0].meta.timestamp),
        "timestamps increase",
    )?;
    check(
        blocks
            .iter()
            .all(|b| b.meta.correlation_id == session.correlation_id),
        "blocks carry the session correlation id",
    )?;

    // Loopback: play the captured audio (mono → the speaker's channel count).
    let ch = usize::from(writer.info().format.delivered.channels);
    let rate_ok = writer.info().format.device.sample_rate == info.format.device.sample_rate;
    if rate_ok {
        for b in &blocks {
            let frames: Vec<f32> = b
                .samples
                .iter()
                .flat_map(|s| std::iter::repeat_n(*s, ch))
                .collect();
            writer.write(frames).await.map_err(err)?;
        }
    }
    engine.stop_audio(info.id).await.map_err(err)?;
    engine
        .update_consent(
            session.id,
            ConsentPurpose::Capture,
            ConsentState::Revoked {
                at: SystemClock::new().wall(),
            },
        )
        .await
        .map_err(err)?;
    let after = reader.recv().await;
    check(after == Err(ReadError::Revoked), "revocation ends capture")?;
    let report = engine.shutdown().await.map_err(err)?;
    check(report.clean, "clean shutdown")?;
    check(
        fake.streams().iter().all(|s| s.is_closed()),
        "every device released",
    )?;
    events.drain_until_closed(Duration::from_secs(5)).await;
    let audio: Vec<&'static str> = events
        .events()
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::Audio(a) => Some(a.kind()),
            _ => None,
        })
        .collect();
    println!("  audio events: {}", audio.join(", "));
    check(
        audio.contains(
            &AudioEvent::CaptureRevoked {
                stream: info.id,
                denial: communication_engine::contracts::audio::AuthorizationDenial::ConsentRevoked,
            }
            .kind(),
        ),
        "revocation published",
    )?;
    Ok(())
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => {
            println!("audio smoke: PASS");
            ExitCode::SUCCESS
        }
        Err(e) => {
            println!("audio smoke: {e}");
            ExitCode::FAILURE
        }
    }
}
