//! Speech smoke example (Phase 4): speech recognition and language
//! observations through the public facade, on the test kit's fake audio
//! backend with a real-time driver thread and the **scripted** ASR adapter
//! (no model, no real recognition: it plays a script).
//!
//! consent → open + start two captures → analysis (scripted detector) on
//! the first → recognition with Phase 3 activity hints on the first and
//! continuous recognition on the second → read capture meanwhile → reset →
//! stop → shutdown. Exits non-zero on any failed expectation (CI gate).
//! Run: `cargo run -p communication-engine --example speech_smoke`

#![allow(clippy::print_stdout)]

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use ce_testkit::EventRecorder;
use ce_testkit::fake_audio::{FakeAudioBackend, FakeDriver};
use communication_engine::audio_intel::ScriptedVad;
use communication_engine::contracts::speech::{
    RecognitionEndReason, SegmentationMode, SpeechEvent,
};
use communication_engine::contracts::time::{Clock, SystemClock};
use communication_engine::prelude::*;
use communication_engine::speech::fake::{ScriptedFactory, UtteranceScript};

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
    let script = vec![
        UtteranceScript::progressive("could you tell me where the train station is", 120),
        UtteranceScript::progressive("thank you very much for your help today", 120),
    ];
    let engine = Engine::builder()
        .audio_backend(Arc::new(fake.clone()))
        .asr_adapter(ScriptedFactory::new(script))
        .build()
        .map_err(err)?;
    let mut events = EventRecorder::new(engine.subscribe().map_err(err)?);
    engine.start().await.map_err(err)?;
    let recognizers = engine.recognizers().await.map_err(err)?;
    println!(
        "  recognizers: {:?}",
        recognizers
            .iter()
            .map(|c| c.adapter.as_str())
            .collect::<Vec<_>>()
    );
    check(recognizers.len() == 1, "adapter discovered without loading")?;
    let session = engine
        .create_session(SessionConfig::default())
        .await
        .map_err(err)?;
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
    let mut readers = Vec::new();
    for _ in 0..2 {
        let r = engine
            .open_capture(session.id, StreamRequest::capture())
            .await
            .map_err(err)?;
        engine.start_audio(r.info().id).await.map_err(err)?;
        readers.push(r);
    }
    let (a, b) = (readers[0].info().id, readers[1].info().id);
    // 1.2 s of activity every 2 s.
    engine
        .start_analysis_with(
            a,
            None,
            Box::new(ScriptedVad::new(|i| if i % 200 < 120 { 0.95 } else { 0.0 })),
        )
        .await
        .map_err(err)?;
    let mut hinted = RecognitionConfig::default();
    hinted.budget.degrade = false;
    let st = engine
        .start_recognition(a, Some(hinted))
        .await
        .map_err(err)?;
    println!(
        "  recognition {} adapter {} config {} lid {:?} code-switching {}",
        a,
        st.recognizer.adapter,
        st.recognizer.config_fingerprint,
        st.capabilities.language_identifier,
        st.capabilities.code_switching
    );
    check(st.running, "hinted recognition running")?;
    let continuous = RecognitionConfig {
        segmentation: SegmentationMode::Continuous,
        ..RecognitionConfig::default()
    };
    engine
        .start_recognition(b, Some(continuous))
        .await
        .map_err(err)?;

    let mut delivered = [0usize; 2];
    let until = tokio::time::Instant::now() + Duration::from_millis(5_000);
    while tokio::time::Instant::now() < until {
        for (k, r) in readers.iter_mut().enumerate() {
            while let Ok(Some(_)) = r.try_recv() {
                delivered[k] += 1;
            }
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    println!("  capture blocks delivered while recognizing: {delivered:?}");
    check(
        delivered.iter().all(|n| *n > 200),
        "capture unaffected by recognition",
    )?;
    let sa = engine.recognition_status(a).await.map_err(err)?;
    println!(
        "  stream a: {} utterances, {} finals, {} revisions, hints {:?}, first partial p50 {} ms",
        sa.counters.utterances,
        sa.counters.finals,
        sa.counters.revisions,
        sa.hints,
        sa.counters.first_partial_ms.p50
    );
    check(sa.counters.finals >= 1, "activity-delimited finals")?;
    engine.reset_recognition(b).await.map_err(err)?;
    let fb = engine.stop_recognition(b).await.map_err(err)?;
    check(
        fb.ended == Some(RecognitionEndReason::Stopped),
        "stop reports its reason",
    )?;
    let report = engine.shutdown().await.map_err(err)?;
    check(report.clean, "clean shutdown")?;
    events.drain_until_closed(Duration::from_secs(5)).await;
    let mut kinds = std::collections::BTreeMap::<&str, usize>::new();
    let mut classes_ok = true;
    let mut sample = None;
    for e in events.events() {
        if let EventPayload::Speech(s) = &e.payload {
            *kinds.entry(s.kind()).or_default() += 1;
            classes_ok &= e.privacy == s.data_class();
            if let SpeechEvent::Transcript(r) = s {
                if r.status == RevisionStatus::Final && r.meta.stream == a {
                    sample.get_or_insert_with(|| (r.text.committed.clone(), r.language.clone()));
                }
            }
        }
    }
    println!("  speech events: {kinds:?}");
    if let Some((text, lang)) = sample {
        println!(
            "  first final on a: {text:?} language {:?}",
            lang.map(|l| l.decision)
        );
    }
    check(classes_ok, "speech events carry their privacy class")?;
    check(
        kinds.get("stopped") == Some(&2),
        "both recognitions reported their end",
    )?;
    check(kinds.get("reset") >= Some(&1), "reset reported")?;
    Ok(())
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => {
            println!("speech smoke: PASS");
            ExitCode::SUCCESS
        }
        Err(e) => {
            println!("speech smoke: {e}");
            ExitCode::FAILURE
        }
    }
}
