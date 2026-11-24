//! Phase 6 end to end through the public facade: fake audio → scripted ASR
//! (Phase 4) → semantic rules (Phase 5, hints) → translation worker →
//! `EventPayload::Translation`. Discovery, lifecycle, source linkage,
//! privacy classes, upstream immutability, start errors and policy, reset
//! and stop withdrawal, fault isolation, stalls, concurrency and shutdown.
//! Deterministic; no network, no model.

mod common;

use std::time::Duration;

use ce_testkit::fake_audio::FakeDriver;
use common::*;
use communication_engine::contracts::config::EngineConfig;
use communication_engine::contracts::semantic::Locality;
use communication_engine::contracts::speech::{LanguageTag, SegmentationMode};
use communication_engine::contracts::translation::{
    PartialPolicy, SourceLanguagePolicy, TranslationEndReason, WithdrawReason,
};
use communication_engine::prelude::*;
use communication_engine::speech::fake::{ScriptedFactory as ScriptedAsr, UtteranceScript};
use communication_engine::translation::{Faults, ScriptedFactory, scripted_capabilities};

fn capture() -> StreamRequest {
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

fn continuous() -> RecognitionConfig {
    let mut c = RecognitionConfig {
        segmentation: SegmentationMode::Continuous,
        partial_interval_ms: 50,
        ..RecognitionConfig::default()
    };
    c.budget.degrade = false;
    c.budget.queue_blocks = 4_000;
    c
}

fn tag(s: &str) -> LanguageTag {
    LanguageTag::parse(s).unwrap()
}

fn en_es(partials: PartialPolicy) -> TranslationConfig {
    TranslationConfig {
        target: Some(tag("es")),
        source: SourceLanguagePolicy::Explicit {
            language: tag("en"),
        },
        partials,
        ..TranslationConfig::default()
    }
}

fn echo() -> ScriptedFactory {
    ScriptedFactory::echo(scripted_capabilities("t.echo", &[("en", "es")]))
}

fn two_utterances() -> ScriptedAsr {
    ScriptedAsr::new(vec![
        UtteranceScript::progressive("the total is forty dollars", 80).endpoint_at(500),
        UtteranceScript::progressive("can you split it", 80).endpoint_at(400),
    ])
}

async fn harness(
    config: EngineConfig,
    asr: ScriptedAsr,
    adapters: Vec<ScriptedFactory>,
) -> AudioHarness {
    audio_harness_custom(config, FakeDriver::Manual, move |mut b| {
        b = b.asr_adapter(asr);
        for a in adapters {
            b = b.translation_adapter(a);
        }
        b
    })
    .await
}

fn translation(h: &AudioHarness) -> Vec<(TranslationEvent, DataClass, Option<SessionId>)> {
    h.events
        .events()
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::Translation(t) => Some((t.clone(), e.privacy, e.session_id)),
            _ => None,
        })
        .collect()
}

async fn wait_tr(
    h: &mut AudioHarness,
    pred: impl Fn(&TranslationEvent) -> bool,
) -> TranslationEvent {
    if let Some((e, _, _)) = translation(h).into_iter().find(|(e, _, _)| pred(e)) {
        return e;
    }
    let e = h
        .events
        .wait_for(
            WAIT,
            |e| matches!(&e.payload, EventPayload::Translation(t) if pred(t)),
        )
        .await;
    let Some(e) = e else {
        let kinds: Vec<&str> = translation(h).iter().map(|(e, _, _)| e.kind()).collect();
        panic!("expected translation event; got {kinds:?}");
    };
    match &e.payload {
        EventPayload::Translation(t) => t.clone(),
        _ => unreachable!(),
    }
}

async fn open(h: &AudioHarness) -> (CaptureReader, AudioStreamId) {
    let reader = h
        .engine
        .open_capture(h.session.id, capture())
        .await
        .unwrap();
    let id = reader.info().id;
    h.engine.start_audio(id).await.unwrap();
    (reader, id)
}

async fn feed(h: &AudioHarness, readers: &mut [&mut CaptureReader], blocks: usize) {
    for _ in 0..blocks {
        h.fake.tick(160);
        for r in readers.iter_mut() {
            let _ = recv_block(r).await;
        }
    }
}

fn final_result(e: &TranslationEvent, utterance: u64) -> bool {
    matches!(e, TranslationEvent::Result(r) if r.source.utterance == utterance && r.state == TranslationState::Final)
}

#[tokio::test]
async fn transcripts_are_translated_end_to_end_with_exact_linkage_and_no_upstream_change() {
    let mut h = harness(audio_config(), two_utterances(), vec![echo()]).await;
    let adapters = h.engine.translation_adapters().await.unwrap();
    assert_eq!(adapters.len(), 1);
    assert_eq!(
        (adapters[0].adapter.as_str(), adapters[0].locality),
        ("t.echo", Locality::InProcess)
    );
    h.grant_capture().await;
    let (mut reader, id) = open(&h).await;
    h.engine.start_semantics(id, None).await.unwrap();
    let st = h
        .engine
        .start_translation(id, Some(en_es(PartialPolicy::Committed)))
        .await
        .unwrap();
    assert!(st.running);
    assert_eq!(st.target, tag("es"));
    h.engine
        .start_recognition(id, Some(continuous()))
        .await
        .unwrap();
    feed(&h, &mut [&mut reader], 120).await;
    wait_tr(&mut h, |e| final_result(e, 1)).await;

    let transcripts: Vec<TranscriptRevision> = h
        .events
        .events()
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::Speech(SpeechEvent::Transcript(r)) => Some((**r).clone()),
            _ => None,
        })
        .collect();
    let all = translation(&h);
    let mut finals = Vec::new();
    for (e, class, session) in &all {
        assert_eq!(*class, e.data_class());
        assert_eq!(*session, Some(h.session.id));
        if let TranslationEvent::Result(r) = e {
            assert_eq!(*class, DataClass::Content);
            // Each result names a transcript revision that was published,
            // with that revision's status and frame range.
            let src = transcripts
                .iter()
                .find(|t| t.meta.utterance == r.source.utterance && t.revision == r.source.revision)
                .expect("source revision was published");
            assert_eq!(src.status, r.source.revision_status);
            assert_eq!(src.meta.frame_range, r.source.frame_range);
            match r.state {
                TranslationState::Final => {
                    assert_eq!(
                        r.text,
                        format!("[es] {}", src.text.committed.to_uppercase())
                    );
                    finals.push(r.source.utterance);
                }
                TranslationState::Stable => assert_eq!(
                    r.text,
                    format!("[es] {}", src.text.committed.to_uppercase())
                ),
                TranslationState::Provisional => {
                    panic!("committed policy never emits provisional results")
                }
            }
        }
    }
    assert_eq!(finals, vec![0, 1]);
    // Phase 5 hints reached translation through the runtime: utterance 1's
    // final was given the final entities of utterance 0 ("forty dollars").
    let hinted = all.iter().any(|(e, _, _)| {
        matches!(e, TranslationEvent::Result(r) if r.source.utterance == 1 && r.state == TranslationState::Final && r.provenance.context_hints >= 1)
    });
    assert!(hinted, "no Phase 5 hint reached the final of utterance 1");
    // Upstream immutability: the transcripts are exactly what the ASR said.
    let final_texts: Vec<&str> = transcripts
        .iter()
        .filter(|t| t.status == RevisionStatus::Final)
        .map(|t| t.text.committed.as_str())
        .collect();
    assert_eq!(
        final_texts,
        vec!["the total is forty dollars", "can you split it"]
    );
    // Operational events never carry content.
    for (e, _, _) in &all {
        if !matches!(e, TranslationEvent::Result(_)) {
            let j = serde_json::to_string(e).unwrap();
            assert!(!j.to_lowercase().contains("forty"), "{j}");
        }
    }
    let st = h.engine.translation_status(id).await.unwrap();
    assert!(
        st.counters.finals == 2 && st.counters.stable >= 1,
        "{:?}",
        st.counters
    );
    assert_eq!(h.engine.metrics().translations_started, 1);
    assert_eq!(h.engine.health().await.unwrap().translations.len(), 1);
    // Recognition stop ends translation (drained, SourceEnded).
    h.engine.stop_recognition(id).await.unwrap();
    wait_tr(&mut h, |e| {
        matches!(
            e,
            TranslationEvent::Stopped {
                reason: TranslationEndReason::SourceEnded,
                ..
            }
        )
    })
    .await;
    let report = h.engine.shutdown().await.unwrap();
    assert!(report.clean);
}

#[tokio::test]
async fn start_errors_and_policy_are_typed() {
    let mut config = audio_config();
    config.translation.max_concurrent = 1;
    let mut caps = scripted_capabilities("t.remote", &[("en", "es")]);
    caps.locality = Locality::Remote;
    let remote = ScriptedFactory::echo(caps);
    let h = harness(config, two_utterances(), vec![remote, echo()]).await;
    h.grant_capture().await;
    let (_r1, a) = open(&h).await;
    let (_r2, b) = open(&h).await;
    let code = |r: Result<TranslationStatus, CeError>| r.unwrap_err().code().to_owned();
    let unknown = AudioStreamId::from_uuid(uuid::Uuid::from_u128(99));
    assert_eq!(
        code(
            h.engine
                .start_translation(unknown, Some(en_es(PartialPolicy::Off)))
                .await
        ),
        "audio.unknown_stream"
    );
    assert_eq!(
        code(h.engine.start_translation(a, None).await),
        "translation.no_target"
    );
    let remote_cfg = TranslationConfig {
        adapter: Some("t.remote".into()),
        ..en_es(PartialPolicy::Off)
    };
    assert_eq!(
        code(h.engine.start_translation(a, Some(remote_cfg)).await),
        "translation.remote_not_allowed"
    );
    // Default selection skips the remote adapter (privacy) and picks the local one.
    let nope = TranslationConfig {
        adapter: Some("nope".into()),
        ..en_es(PartialPolicy::Off)
    };
    assert_eq!(
        code(h.engine.start_translation(a, Some(nope)).await),
        "translation.no_adapter"
    );
    let term = TranslationConfig {
        terminology: Some("missing".into()),
        ..en_es(PartialPolicy::Off)
    };
    assert_eq!(
        code(h.engine.start_translation(a, Some(term)).await),
        "translation.unknown_terminology"
    );
    let mut bad = en_es(PartialPolicy::Off);
    bad.budget.queue = 1;
    assert_eq!(
        code(h.engine.start_translation(a, Some(bad)).await),
        "config.translation_invalid"
    );
    let st = h
        .engine
        .start_translation(a, Some(en_es(PartialPolicy::Off)))
        .await
        .unwrap();
    assert_eq!(
        st.capabilities.adapter, "t.echo",
        "remote adapter never selected implicitly"
    );
    assert_eq!(
        code(
            h.engine
                .start_translation(a, Some(en_es(PartialPolicy::Off)))
                .await
        ),
        "translation.already_running"
    );
    assert_eq!(
        code(
            h.engine
                .start_translation(b, Some(en_es(PartialPolicy::Off)))
                .await
        ),
        "translation.capacity_exhausted"
    );
    let fin = h.engine.stop_translation(a).await.unwrap();
    assert_eq!(fin.ended, Some(TranslationEndReason::Stopped));
    assert_eq!(
        code(h.engine.translation_status(unknown).await),
        "translation.not_found"
    );
}

#[tokio::test]
async fn reset_and_stop_withdraw_open_results() {
    let asr = ScriptedAsr::new(vec![
        UtteranceScript::progressive("one two three four five six seven eight nine ten", 60)
            .endpoint_at(5_000),
    ]);
    let mut h = harness(audio_config(), asr, vec![echo()]).await;
    h.grant_capture().await;
    let (mut reader, id) = open(&h).await;
    h.engine
        .start_translation(id, Some(en_es(PartialPolicy::Full)))
        .await
        .unwrap();
    h.engine
        .start_recognition(id, Some(continuous()))
        .await
        .unwrap();
    feed(&h, &mut [&mut reader], 30).await;
    wait_tr(
        &mut h,
        |e| matches!(e, TranslationEvent::Result(r) if r.state == TranslationState::Provisional),
    )
    .await;
    h.engine.reset_translation(id).await.unwrap();
    wait_tr(&mut h, |e| {
        matches!(
            e,
            TranslationEvent::Withdrawn {
                reason: WithdrawReason::Reset,
                ..
            }
        )
    })
    .await;
    feed(&h, &mut [&mut reader], 20).await;
    h.engine.stop_translation(id).await.unwrap();
    wait_tr(&mut h, |e| {
        matches!(
            e,
            TranslationEvent::Stopped {
                reason: TranslationEndReason::Stopped,
                ..
            }
        )
    })
    .await;
    let evs = translation(&h);
    let last_result = evs
        .iter()
        .rposition(|(e, _, _)| matches!(e, TranslationEvent::Result(_)));
    let last_withdraw = evs
        .iter()
        .rposition(|(e, _, _)| matches!(e, TranslationEvent::Withdrawn { .. }));
    assert!(
        last_withdraw > last_result,
        "no unfinished result is left current after stop: {:?}",
        evs.iter().map(|e| e.0.kind()).collect::<Vec<_>>()
    );
    // Capture and recognition continue.
    feed(&h, &mut [&mut reader], 5).await;
    assert!(h.engine.recognition_status(id).await.unwrap().running);
}

#[tokio::test]
async fn a_failing_adapter_is_contained_and_reported() {
    let panicking = ScriptedFactory::echo(scripted_capabilities("t.panic", &[("en", "es")]))
        .with_faults(Faults {
            panic_at: Some(0),
            ..Faults::default()
        });
    let mut h = harness(audio_config(), two_utterances(), vec![panicking]).await;
    h.grant_capture().await;
    let (mut reader, id) = open(&h).await;
    h.engine
        .start_translation(id, Some(en_es(PartialPolicy::Off)))
        .await
        .unwrap();
    h.engine
        .start_recognition(id, Some(continuous()))
        .await
        .unwrap();
    feed(&h, &mut [&mut reader], 80).await;
    let f = wait_tr(&mut h, |e| matches!(e, TranslationEvent::Failed { .. })).await;
    let TranslationEvent::Failed { error, .. } = f else {
        unreachable!()
    };
    assert_eq!(error.code, "translation.worker_panicked");
    wait_tr(&mut h, |e| {
        matches!(
            e,
            TranslationEvent::Stopped {
                reason: TranslationEndReason::Failed,
                ..
            }
        )
    })
    .await;
    feed(&h, &mut [&mut reader], 40).await;
    assert!(
        h.engine.recognition_status(id).await.unwrap().running,
        "recognition unaffected"
    );
    assert_eq!(h.engine.metrics().translation_failures, 1);
    // A new translation can start on the same stream.
    let mut h2 = h;
    let st = h2.engine.translation_status(id).await.unwrap();
    assert_eq!(st.ended, Some(TranslationEndReason::Failed));
    let _ = &mut h2;
}

#[tokio::test]
async fn a_hung_adapter_is_detected_as_stalled() {
    let mut config = audio_config();
    config.translation.translation.budget.stall_timeout_ms = 200;
    let hung = ScriptedFactory::echo(scripted_capabilities("t.hang", &[("en", "es")])).with_faults(
        Faults {
            hang_at: Some((0, Duration::from_millis(1_500))),
            ..Faults::default()
        },
    );
    let mut h = harness(config, two_utterances(), vec![hung]).await;
    h.grant_capture().await;
    let (mut reader, id) = open(&h).await;
    let mut c = en_es(PartialPolicy::Off);
    c.budget.stall_timeout_ms = 200;
    h.engine.start_translation(id, Some(c)).await.unwrap();
    h.engine
        .start_recognition(id, Some(continuous()))
        .await
        .unwrap();
    feed(&h, &mut [&mut reader], 80).await;
    let f = wait_tr(&mut h, |e| matches!(e, TranslationEvent::Failed { .. })).await;
    let TranslationEvent::Failed { error, .. } = f else {
        unreachable!()
    };
    assert_eq!(error.code, "translation.stalled");
    let st = h.engine.translation_status(id).await.unwrap();
    assert_eq!(
        (st.running, st.ended),
        (false, Some(TranslationEndReason::Failed))
    );
    assert!(
        h.engine.shutdown().await.unwrap().clean,
        "the abandoned worker exits within the shutdown budget"
    );
}

#[tokio::test]
async fn streams_are_isolated_and_shutdown_drains() {
    let panicking = ScriptedFactory::echo(scripted_capabilities("t.panic", &[("en", "fr")]))
        .with_faults(Faults {
            panic_at: Some(0),
            ..Faults::default()
        });
    let mut h = harness(audio_config(), two_utterances(), vec![echo(), panicking]).await;
    h.grant_capture().await;
    let (mut ra, a) = open(&h).await;
    let (mut rb, b) = open(&h).await;
    h.engine
        .start_translation(a, Some(en_es(PartialPolicy::Off)))
        .await
        .unwrap();
    let fr = TranslationConfig {
        target: Some(tag("fr")),
        adapter: Some("t.panic".into()),
        ..en_es(PartialPolicy::Off)
    };
    h.engine.start_translation(b, Some(fr)).await.unwrap();
    for s in [a, b] {
        h.engine
            .start_recognition(s, Some(continuous()))
            .await
            .unwrap();
    }
    feed(&h, &mut [&mut ra, &mut rb], 120).await;
    wait_tr(&mut h, |e| final_result(e, 1)).await;
    wait_tr(
        &mut h,
        |e| matches!(e, TranslationEvent::Failed { stream, .. } if *stream == b),
    )
    .await;
    let st = h.engine.translation_status(a).await.unwrap();
    assert!(
        st.running && st.counters.finals == 2,
        "stream a unaffected by b's failure: {:?}",
        st.counters
    );
    let AudioHarness {
        engine, mut events, ..
    } = h;
    let report = engine.shutdown().await.unwrap();
    assert!(report.clean, "{report:?}");
    events.drain_until_closed(WAIT).await;
    // Shutdown stops recognitions first, which ends translation with
    // `source_ended` (first stop request wins); both reasons drain queued
    // transcripts, so stream a's finals were all translated.
    let end = events.events().iter().find_map(|e| match &e.payload {
        EventPayload::Translation(TranslationEvent::Stopped { stream, reason, .. })
            if *stream == a =>
        {
            Some(*reason)
        }
        _ => None,
    });
    assert!(
        matches!(
            end,
            Some(TranslationEndReason::SourceEnded | TranslationEndReason::Shutdown)
        ),
        "{end:?}"
    );
}

#[tokio::test]
async fn logs_health_and_metrics_carry_no_translation_content() {
    let sink = std::sync::Arc::new(ce_testkit::RecordingSink::default());
    let s2 = sink.clone();
    let mut h = audio_harness_custom(audio_config(), FakeDriver::Manual, move |b| {
        b.asr_adapter(two_utterances())
            .translation_adapter(echo())
            .observability(s2)
    })
    .await;
    h.grant_capture().await;
    let (mut reader, id) = open(&h).await;
    h.engine.start_semantics(id, None).await.unwrap();
    h.engine
        .start_translation(id, Some(en_es(PartialPolicy::Full)))
        .await
        .unwrap();
    h.engine
        .start_recognition(id, Some(continuous()))
        .await
        .unwrap();
    feed(&h, &mut [&mut reader], 120).await;
    wait_tr(&mut h, |e| final_result(e, 1)).await;
    h.engine.stop_recognition(id).await.unwrap();
    wait_tr(&mut h, |e| matches!(e, TranslationEvent::Stopped { .. })).await;
    let logs = sink.rendered();
    assert!(
        logs.contains("translation.started") && logs.contains("translation.ended"),
        "{logs}"
    );
    // Source words and their (uppercased) translations never appear.
    for word in ["forty", "dollars", "split", "total"] {
        assert!(
            !logs.to_lowercase().contains(word),
            "{word} leaked into logs"
        );
    }
    let health = serde_json::to_string(&h.engine.health().await.unwrap()).unwrap();
    for word in ["FORTY", "SPLIT", "forty", "split"] {
        assert!(!health.contains(word), "{word} leaked into health");
    }
    let metrics = serde_json::to_string(&h.engine.metrics()).unwrap();
    assert!(!metrics.to_lowercase().contains("forty"));
    h.engine.shutdown().await.unwrap();
}
