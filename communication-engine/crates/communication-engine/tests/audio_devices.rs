//! Device discovery: catalog contents, opaque ids, hotplug and default
//! changes, invalid backend responses, enumeration failures and timeouts.

mod common;

use std::time::Duration;

use ce_testkit::fake_audio::{FakeDriver, device};
use common::*;
use communication_engine::contracts::audio::{
    AudioDirection, AudioEvent, BufferSizeRange, SupportedFormatRange,
};
use communication_engine::prelude::*;

#[tokio::test]
async fn the_catalog_reports_devices_formats_and_defaults_with_opaque_ids() {
    let mut h = audio_harness().await;
    let catalog = h.engine.audio_devices().await.unwrap();
    assert_eq!(catalog.backend.name, "fake");
    assert_eq!(catalog.devices.len(), 2);
    let mic = catalog.inputs().next().unwrap();
    let speaker = catalog.outputs().next().unwrap();
    assert_eq!(
        (catalog.default_input, catalog.default_output),
        (Some(mic.id), Some(speaker.id))
    );
    assert_eq!(mic.name, "Fake mic");
    assert!(mic.supports_input && !mic.supports_output && mic.is_default_input);
    assert_eq!(mic.input_formats.len(), 2);
    assert_eq!(mic.default_input_format.unwrap().sample_rate, 48_000);
    assert!(mic.id.to_string().starts_with("dev_"));
    // Ids are stable across re-enumeration.
    let again = h.engine.audio_devices().await.unwrap();
    assert_eq!(
        again.devices.iter().map(|d| d.id).collect::<Vec<_>>(),
        catalog.devices.iter().map(|d| d.id).collect::<Vec<_>>()
    );
    h.wait_audio(|e| {
        matches!(
            e,
            AudioEvent::DevicesEnumerated {
                inputs: 1,
                outputs: 1
            }
        )
    })
    .await;
    // A specific device can be opened by id; a capture-only device cannot play.
    h.engine
        .open_playback(
            h.session.id,
            StreamRequest {
                device: DeviceSelector::Device(speaker.id),
                ..StreamRequest::playback()
            },
        )
        .await
        .unwrap();
    let err = h
        .engine
        .open_playback(
            h.session.id,
            StreamRequest {
                device: DeviceSelector::Device(mic.id),
                ..StreamRequest::playback()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err.code(), "audio.device_direction_unsupported");
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn hotplug_and_default_changes_are_detected_and_published() {
    let mut h = audio_harness().await;
    let first = h.engine.audio_devices().await.unwrap();
    let mic = first.default_input.unwrap();
    // The fake notifies; the core re-enumerates and diffs.
    let mut headset = device("headset", true, true);
    headset.name = "Alex's headset".into();
    h.fake.add_device(headset);
    let added = h
        .wait_audio(|e| matches!(e, AudioEvent::DeviceAdded { .. }))
        .await;
    let AudioEvent::DeviceAdded { device: headset_id } = added else {
        unreachable!()
    };
    h.fake.set_default_input(Some("headset"));
    let changed = h
        .wait_audio(|e| {
            matches!(
                e,
                AudioEvent::DefaultDeviceChanged {
                    direction: AudioDirection::Capture,
                    ..
                }
            )
        })
        .await;
    assert_eq!(
        changed,
        AudioEvent::DefaultDeviceChanged {
            direction: AudioDirection::Capture,
            from: Some(mic),
            to: Some(headset_id)
        }
    );
    h.fake.remove_device("headset");
    h.wait_audio(|e| *e == AudioEvent::DeviceRemoved { device: headset_id })
        .await;
    let catalog = h.engine.audio_devices().await.unwrap();
    assert!(catalog.device(headset_id).is_none());
    assert_eq!(
        catalog.default_input, None,
        "the default went away with the device"
    );
    // Re-adding the same backend key reuses the same CE id.
    h.fake.add_device(device("headset", true, true));
    h.wait_audio(|e| *e == AudioEvent::DeviceAdded { device: headset_id })
        .await;
    h.engine.shutdown().await.unwrap();
    // Device names never appear in events.
    h.events.drain_until_closed(WAIT).await;
    for e in h.events.events() {
        let json = serde_json::to_string(e.as_ref()).unwrap();
        assert!(
            !json.contains("Alex") && !json.contains("Fake mic"),
            "{json}"
        );
    }
}

#[tokio::test]
async fn device_polling_detects_changes_without_notifications() {
    let mut config = audio_config();
    config.audio.device_poll_interval_ms = Some(100);
    let mut h = audio_harness_with(config, FakeDriver::Manual).await;
    h.engine.audio_devices().await.unwrap();
    let before = h.fake.enumerations();
    tokio::time::timeout(WAIT, async {
        while h.fake.enumerations() < before + 2 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("periodic enumeration");
    let _ = h
        .wait_audio(|e| matches!(e, AudioEvent::DevicesEnumerated { .. }))
        .await;
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn invalid_backend_responses_are_skipped_and_reported() {
    let h = audio_harness().await;
    let mut weird = device("weird", true, false);
    let bad = |ch, min, max| SupportedFormatRange {
        channels: ch,
        min_sample_rate: min,
        max_sample_rate: max,
        sample_format: SampleFormat::I16,
        buffer: BufferSizeRange::Unknown,
    };
    weird.input_formats = vec![
        bad(0, 8_000, 48_000),
        bad(2, 48_000, 8_000),
        bad(1, 1, u32::MAX),
    ];
    weird.skipped_formats = 3;
    let mut broken = device("broken", true, false);
    broken.input_formats = vec![bad(0, 1, 2)];
    h.fake.add_device(weird);
    h.fake.add_device(broken);
    let catalog = h.engine.audio_devices().await.unwrap();
    let w = catalog
        .devices
        .iter()
        .find(|d| d.name == "Fake weird")
        .unwrap();
    assert_eq!(
        w.input_formats.len(),
        1,
        "only the sanitizable range survives"
    );
    assert_eq!(
        (
            w.input_formats[0].min_sample_rate,
            w.input_formats[0].max_sample_rate
        ),
        (1_000, 768_000)
    );
    assert_eq!(w.skipped_formats, 3 + 2);
    assert!(catalog.devices.iter().all(|d| d.name != "Fake broken"));
    assert!(
        catalog
            .problems
            .iter()
            .any(|p| p.code == "audio.device_unusable")
    );
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn enumeration_failures_and_timeouts_are_typed_and_keep_the_previous_catalog() {
    let mut config = audio_config();
    config.audio.enumerate_timeout_ms = 100;
    let mut h = audio_harness_with(config, FakeDriver::Manual).await;
    h.engine.audio_devices().await.unwrap();
    h.fake.fail_enumerations(1);
    let err = h.engine.audio_devices().await.unwrap_err();
    assert_eq!(err.code(), "fake.enumeration_failed");
    h.wait_audio(|e| matches!(e, AudioEvent::EnumerationFailed { .. }))
        .await;
    // Opening needs a fresh enumeration; when it fails the open fails too.
    h.fake.fail_enumerations(1);
    assert_eq!(
        h.engine
            .open_playback(h.session.id, StreamRequest::playback())
            .await
            .unwrap_err()
            .code(),
        "fake.enumeration_failed"
    );
    h.fake.delay_enumeration(Duration::from_millis(500));
    let err = h.engine.audio_devices().await.unwrap_err();
    assert_eq!(
        (err.category(), err.code()),
        (ErrorCategory::Timeout, "audio.enumeration_timeout")
    );
    h.fake.delay_enumeration(Duration::ZERO);
    assert_eq!(h.engine.audio_devices().await.unwrap().devices.len(), 2);
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn missing_devices_are_typed_errors() {
    let h = audio_harness().await;
    h.fake.set_default_output(None);
    let err = h
        .engine
        .open_playback(h.session.id, StreamRequest::playback())
        .await
        .unwrap_err();
    assert_eq!(
        (err.category(), err.code()),
        (ErrorCategory::Device, "audio.no_default_device")
    );
    let stale = AudioDeviceId(4242);
    let err = h
        .engine
        .open_playback(
            h.session.id,
            StreamRequest {
                device: DeviceSelector::Device(stale),
                ..StreamRequest::playback()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err.code(), "audio.device_not_found");
    h.engine.shutdown().await.unwrap();
}
