//! Tests that need the real model (`CE_SHERPA_MODEL_DIR`). They are
//! `#[ignore]`d so a plain `cargo test` never pretends to have run them;
//! run with `cargo test -- --ignored`. A missing model is a failure there,
//! never a silent skip.

use std::sync::Arc;
use std::time::Duration;

use ce_asr_sherpa::{SherpaConfig, SherpaFactory, model_dir_from_env};
use ce_core::speech::LanguageHint;
use ce_speech::adapter::AsrFactory;
use ce_speech::eval::{self, Split};
use ce_testkit::fake_audio::{FakeAudioBackend, FakeDriver};
use ce_testkit::{EventRecorder, ManualClock, SequentialIds};
use communication_engine::contracts::speech::{RecognitionEndReason, SegmentationMode};
use communication_engine::contracts::time::Clock;
use communication_engine::prelude::*;

fn factory() -> SherpaFactory {
    let dir = model_dir_from_env().expect("CE_SHERPA_MODEL_DIR must point at the model");
    SherpaFactory::new(SherpaConfig::zipformer_en_2023_06_26_int8(dir))
}

#[test]
#[ignore = "needs the sherpa-onnx model (CE_SHERPA_MODEL_DIR)"]
fn recognizes_a_synthetic_fixture_and_reuses_the_loaded_model() {
    let f = factory();
    let caps = f.capabilities();
    assert!(caps.model.as_ref().unwrap().size_bytes.unwrap() > 60_000_000);
    let mut a = f.create().unwrap();
    assert!(!a.load().unwrap().reused);
    let mut b = f.create().unwrap();
    assert!(b.load().unwrap().reused, "second adapter shares the model");
    let (_, samples) = eval::load_split(Split::Dev)
        .unwrap()
        .into_iter()
        .find(|(m, _)| m.id == "dev_kal16_station")
        .unwrap();
    a.begin(0, &LanguageHint::Auto).unwrap();
    let mut partials = 0;
    for c in samples.chunks(1_600) {
        if a.accept(c).unwrap().is_some() {
            partials += 1;
        }
    }
    let fin = a.finalize().unwrap();
    assert!(partials >= 3, "streaming partials: {partials}");
    assert!(
        fin.text.contains("TRAIN STATION"),
        "unexpected text: {}",
        fin.text
    );
    // Same input → same output (num_threads = 1).
    b.begin(0, &LanguageHint::Auto).unwrap();
    for c in samples.chunks(1_600) {
        b.accept(c).unwrap();
    }
    assert_eq!(b.finalize().unwrap().text, fin.text);
    // Non-English hints are refused, not ignored.
    let fr = LanguageHint::Fixed(ce_core::speech::LanguageTag::parse("fr").unwrap());
    assert_eq!(
        a.begin(1, &fr).unwrap_err().code(),
        "asr.language_unsupported"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs the sherpa-onnx model (CE_SHERPA_MODEL_DIR)"]
async fn the_real_model_runs_inside_the_engine_on_the_fake_backend() {
    let fake = FakeAudioBackend::new(FakeDriver::Manual);
    let clock = Arc::new(ManualClock::default());
    let engine = Engine::builder()
        .clock(clock.clone())
        .id_generator(Arc::new(SequentialIds::default()))
        .audio_backend(Arc::new(fake.clone()))
        .asr_adapter(factory())
        .build()
        .unwrap();
    let mut events = EventRecorder::new(engine.subscribe().unwrap());
    engine.start().await.unwrap();
    let caps = engine.recognizers().await.unwrap();
    assert_eq!(caps[0].adapter, ce_asr_sherpa::ADAPTER_ID);
    let session = engine
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    engine
        .update_consent(
            session.id,
            ConsentPurpose::Capture,
            ConsentState::Granted { at: clock.wall() },
        )
        .await
        .unwrap();
    let request = StreamRequest {
        format: FormatPreference {
            sample_rate: Some(48_000),
            channels: Some(2),
            sample_format: None,
            buffer_frames: None,
        },
        ..StreamRequest::capture()
    };
    let mut reader = engine.open_capture(session.id, request).await.unwrap();
    let id = reader.info().id;
    engine.start_audio(id).await.unwrap();
    let cfg = RecognitionConfig {
        segmentation: SegmentationMode::Continuous,
        ..RecognitionConfig::default()
    };
    engine.start_recognition(id, Some(cfg)).await.unwrap();
    let ready = events
        .wait_for(Duration::from_secs(30), |e| {
            matches!(
                &e.payload,
                EventPayload::Speech(SpeechEvent::ModelReady { .. })
            )
        })
        .await
        .expect("model loads on the worker");
    let EventPayload::Speech(SpeechEvent::ModelReady { load_ms, .. }) = &ready.payload else {
        unreachable!()
    };
    assert!(*load_ms > 0);
    // 3 s of the fake backend's test signal (not speech) at 48 kHz stereo:
    // capture keeps flowing while the real model decodes it.
    for _ in 0..300 {
        fake.tick(480);
        tokio::time::timeout(Duration::from_secs(5), reader.recv())
            .await
            .expect("capture is not delayed")
            .expect("block");
    }
    let st = engine.stop_recognition(id).await.unwrap();
    assert_eq!(st.ended, Some(RecognitionEndReason::Stopped));
    assert!(st.loaded && st.counters.blocks > 0, "{:?}", st.counters);
    assert!(engine.shutdown().await.unwrap().clean);
}
