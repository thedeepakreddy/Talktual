//! Bounded buffering: device-ring overflow with exact gap placement,
//! delivery-queue policies, bounded memory, playback backpressure,
//! underruns and playback content.

mod common;

use std::time::Duration;

use ce_testkit::fake_audio::{FakeDriver, expected_sample};
use common::*;
use communication_engine::contracts::audio::{
    AudioEvent, DeliveryOverflowPolicy, DiscontinuityKind, OverflowStage,
};
use communication_engine::contracts::resource::QueueCapacity;
use communication_engine::prelude::*;

const TOL: f32 = 1.0 / 32_768.0;

#[tokio::test]
async fn device_ring_overflow_drops_newest_frames_and_flags_the_exact_gap() {
    let mut config = audio_config();
    config.audio.capture_ring_ms = 20; // 960 frames at 48 kHz
    let mut h = audio_harness_with(config, FakeDriver::Manual).await;
    h.grant_capture().await;
    let mut reader = h
        .engine
        .open_capture(h.session.id, StreamRequest::capture())
        .await
        .unwrap();
    h.engine.start_audio(reader.info().id).await.unwrap();
    // One huge callback: only what fits the ring is kept.
    h.fake.tick(5_000);
    let mut blocks = Vec::new();
    while blocks
        .iter()
        .map(|b: &AudioBlock| u64::from(b.meta.frames))
        .sum::<u64>()
        < 960
    {
        blocks.push(recv_block(&mut reader).await);
    }
    // The ring has been drained; the next callback fits entirely.
    h.fake.tick(480);
    blocks.push(recv_block(&mut reader).await);
    let gap = blocks
        .iter()
        .find(|b| b.meta.discontinuity.is_some())
        .expect("a flagged block");
    let d = gap.meta.discontinuity.unwrap();
    assert_eq!(d.kind, DiscontinuityKind::DeviceOverflow);
    assert_eq!(d.lost_frames, Some(5_000 - 960));
    assert_eq!(
        gap.meta.frame_index, 5_000,
        "the sample clock skips exactly the lost frames"
    );
    // The data after the gap is the device's frame 5000 onward.
    for (k, s) in gap.samples.iter().enumerate() {
        assert!((s - expected_sample(5_000 + k as u64, 0)).abs() <= TOL);
    }
    // Blocks before the gap are contiguous and end exactly at 960.
    let before: u64 = blocks
        .iter()
        .take_while(|b| b.meta.discontinuity.is_none())
        .map(|b| u64::from(b.meta.frames))
        .sum();
    assert_eq!(before, 960);
    let e = h
        .wait_audio(|e| {
            matches!(
                e,
                AudioEvent::Overflow {
                    stage: OverflowStage::DeviceRing,
                    ..
                }
            )
        })
        .await;
    assert_eq!(
        e,
        AudioEvent::Overflow {
            stream: reader.info().id,
            stage: OverflowStage::DeviceRing,
            lost_frames_total: 4_040
        }
    );
    h.wait_audio(|e| {
        matches!(
            e,
            AudioEvent::Discontinuity {
                kind: DiscontinuityKind::DeviceOverflow,
                ..
            }
        )
    })
    .await;
    h.engine.shutdown().await.unwrap();
}

async fn fill_delivery_queue(policy: DeliveryOverflowPolicy) -> (AudioHarness, CaptureReader) {
    let mut config = audio_config();
    config.audio.capture_queue_blocks = QueueCapacity::new(2).unwrap();
    config.audio.capture_queue_overflow = policy;
    let h = audio_harness_with(config, FakeDriver::Manual).await;
    h.grant_capture().await;
    let reader = h
        .engine
        .open_capture(h.session.id, StreamRequest::capture())
        .await
        .unwrap();
    h.engine.start_audio(reader.info().id).await.unwrap();
    for _ in 0..6 {
        h.fake.tick(480);
        // Let the worker turn each callback into a block.
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    tokio::time::timeout(WAIT, async {
        loop {
            let s = h.engine.audio_streams().await.unwrap();
            if s[0].counters.queue_dropped_blocks == 4 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("four blocks dropped");
    assert!(
        reader.queued() <= 2,
        "memory is bounded by the queue capacity"
    );
    (h, reader)
}

#[tokio::test]
async fn drop_oldest_keeps_the_newest_audio_and_flags_the_block_after_the_gap() {
    let (mut h, mut reader) = fill_delivery_queue(DeliveryOverflowPolicy::DropOldest).await;
    let a = recv_block(&mut reader).await;
    let b = recv_block(&mut reader).await;
    assert_eq!(
        (a.meta.sequence, b.meta.sequence),
        (4, 5),
        "sequence gap reveals the drops"
    );
    let d = a.meta.discontinuity.unwrap();
    assert_eq!(
        (d.kind, d.lost_frames),
        (DiscontinuityKind::DeliveryOverflow, Some(4 * 480))
    );
    assert_eq!(b.meta.discontinuity, None);
    h.wait_audio(|e| {
        matches!(
            e,
            AudioEvent::Overflow {
                stage: OverflowStage::DeliveryQueue,
                ..
            }
        )
    })
    .await;
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn drop_newest_keeps_the_queued_prefix_and_flags_the_next_delivered_block() {
    let (h, mut reader) = fill_delivery_queue(DeliveryOverflowPolicy::DropNewest).await;
    let a = recv_block(&mut reader).await;
    let b = recv_block(&mut reader).await;
    assert_eq!((a.meta.sequence, b.meta.sequence), (0, 1));
    assert!(a.meta.discontinuity.is_none() && b.meta.discontinuity.is_none());
    h.fake.tick(480);
    let c = recv_block(&mut reader).await;
    assert_eq!(c.meta.sequence, 6);
    let d = c.meta.discontinuity.unwrap();
    assert_eq!(
        (d.kind, d.lost_frames),
        (DiscontinuityKind::DeliveryOverflow, Some(4 * 480))
    );
    assert_eq!(c.meta.frame_index, 6 * 480);
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn playback_writes_are_bounded_with_backpressure_and_played_in_order() {
    let mut config = audio_config();
    config.audio.playback_queue_blocks = QueueCapacity::new(2).unwrap();
    config.audio.playback_ring_ms = 20; // 960 frames at 48 kHz
    let h = audio_harness_with(config, FakeDriver::Manual).await;
    let mut writer = h
        .engine
        .open_playback(h.session.id, StreamRequest::playback())
        .await
        .unwrap();
    let ch = usize::from(writer.info().format.delivered.channels);
    assert_eq!(ch, 2);
    let block = |start: u64| -> Vec<f32> {
        (0..480u64)
            .flat_map(|f| (0..ch).map(move |c| expected_sample(start + f, c)))
            .collect()
    };
    h.engine.start_audio(writer.info().id).await.unwrap();
    // Validation.
    assert_eq!(
        writer.try_write(vec![0.1; 3]).unwrap_err().code(),
        "audio.invalid_playback_block"
    );
    assert_eq!(
        writer.try_write(Vec::new()).unwrap_err().code(),
        "audio.invalid_playback_block"
    );
    let too_big = vec![0.0; (writer.info().block_frames as usize + 1) * ch];
    assert_eq!(
        writer.try_write(too_big).unwrap_err().code(),
        "audio.invalid_playback_block"
    );
    // Fill ring (2 blocks) + queue (2 blocks), then the queue is full.
    let mut accepted = 0u64;
    let mut saturated = None;
    for i in 0..8u64 {
        match writer.try_write(block(i * 480)) {
            Ok(seq) => {
                assert_eq!(seq, accepted);
                accepted += 1;
            }
            Err(e) => {
                saturated = Some(e);
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    let e = saturated.expect("the bounded queue rejects");
    assert_eq!(
        (e.category(), e.code()),
        (ErrorCategory::QueueSaturated, "queue.saturated")
    );
    // An awaiting write proceeds once the device consumes data.
    let pending = {
        let next = block(accepted * 480);
        async move {
            let seq = writer.write(next).await.unwrap();
            (writer, seq)
        }
    };
    let consumer = async {
        for _ in 0..4 {
            tokio::time::sleep(Duration::from_millis(15)).await;
            h.fake.tick(480);
        }
    };
    let ((writer, seq), ()) = tokio::join!(pending, consumer);
    assert_eq!(seq, accepted);
    let played = h.fake.last_stream().unwrap().played();
    assert_eq!(played.len(), 4 * 480 * ch);
    for (k, s) in played.iter().enumerate() {
        let (f, c) = ((k / ch) as u64, k % ch);
        assert!((s - expected_sample(f, c)).abs() <= TOL, "sample {k}");
    }
    let id = writer.info().id;
    let status = h.engine.close_audio(id).await.unwrap();
    assert_eq!(status.counters.frames, 4 * 480);
    let mut writer = writer;
    assert_eq!(
        writer.try_write(block(0)).unwrap_err().code(),
        "audio.stream_closed"
    );
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn underruns_are_counted_only_when_data_was_pending() {
    let mut config = audio_config();
    config.audio.playback_ring_ms = 20; // 960 frames
    let mut h = audio_harness_with(config, FakeDriver::Manual).await;
    let mut writer = h
        .engine
        .open_playback(h.session.id, StreamRequest::playback())
        .await
        .unwrap();
    let id = writer.info().id;
    h.engine.start_audio(id).await.unwrap();
    // Idle: nothing written, silence is not an underrun.
    h.fake.tick(480);
    // One large write (2000 frames) exceeds the ring; consume faster than
    // the worker can refill within a single callback.
    writer.write(vec![0.25; 2_000 * 2]).await.unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    h.fake.tick(1_500);
    let e = h
        .wait_audio(|e| matches!(e, AudioEvent::Underflow { .. }))
        .await;
    let AudioEvent::Underflow {
        missing_frames_total,
        ..
    } = e
    else {
        unreachable!()
    };
    assert_eq!(missing_frames_total, 1_500 - 960);
    let played = h.fake.last_stream().unwrap().played();
    assert!(
        played[..480 * 2].iter().all(|s| *s == 0.0),
        "idle silence first"
    );
    h.engine.shutdown().await.unwrap();
}
