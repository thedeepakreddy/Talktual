//! Phase 5 end to end through the public facade: fake audio → scripted
//! ASR (Phase 4) → semantic worker → `EventPayload::Semantic`. Discovery,
//! lifecycle, privacy classes, transcript linkage, references across
//! utterances, confirmation, context reset, session close, policy, start
//! errors, isolation between streams, and shutdown. Deterministic; no
//! network, no model.

mod common;

use std::time::Duration;

use ce_testkit::fake_audio::FakeDriver;
use common::*;
use communication_engine::contracts::config::EngineConfig;
use communication_engine::contracts::semantic::{
    ContextChange, Epistemic, Locality, ObservationStatus, Resolution, SemanticEndReason,
};
use communication_engine::contracts::speech::SegmentationMode;
use communication_engine::prelude::*;
use communication_engine::semantic::scripted::{Script, ScriptedFactory as ScriptedSemantic};
use communication_engine::speech::fake::{ScriptedFactory, UtteranceScript};

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

async fn harness(
    config: EngineConfig,
    asr: ScriptedFactory,
    extra: Option<ScriptedSemantic>,
) -> AudioHarness {
    audio_harness_custom(config, FakeDriver::Manual, move |mut b| {
        b = b.asr_adapter(asr);
        if let Some(s) = extra {
            b = b.semantic_adapter(s);
        }
        b
    })
    .await
}

fn semantic(h: &AudioHarness) -> Vec<(SemanticEvent, DataClass, Option<SessionId>)> {
    h.events
        .events()
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::Semantic(s) => Some((s.clone(), e.privacy, e.session_id)),
            _ => None,
        })
        .collect()
}

async fn wait_semantic(
    h: &mut AudioHarness,
    pred: impl Fn(&SemanticEvent) -> bool,
) -> SemanticEvent {
    if let Some((e, _, _)) = semantic(h).into_iter().find(|(e, _, _)| pred(e)) {
        return e;
    }
    let e = h
        .events
        .wait_for(
            WAIT,
            |e| matches!(&e.payload, EventPayload::Semantic(s) if pred(s)),
        )
        .await;
    let Some(e) = e else {
        let kinds: Vec<&str> = semantic(h).iter().map(|(e, _, _)| e.kind()).collect();
        panic!("expected semantic event; got {kinds:?}");
    };
    match &e.payload {
        EventPayload::Semantic(s) => s.clone(),
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

fn two_utterances() -> ScriptedFactory {
    ScriptedFactory::new(vec![
        UtteranceScript::progressive("the total is forty dollars", 80).endpoint_at(500),
        UtteranceScript::progressive("can you split it", 80).endpoint_at(400),
    ])
}

#[tokio::test]
async fn transcripts_become_linked_observations_end_to_end() {
    let mut h = harness(audio_config(), two_utterances(), None).await;
    let adapters = h.engine.semantic_adapters().await.unwrap();
    assert_eq!(adapters.len(), 1);
    assert_eq!(adapters[0].adapter, "ce.semantic.rules");
    assert_eq!(adapters[0].locality, Locality::InProcess);
    h.grant_capture().await;
    let (mut reader, id) = open(&h).await;
    // Semantics may start before recognition.
    let st = h.engine.start_semantics(id, None).await.unwrap();
    assert!(st.running);
    h.engine
        .start_recognition(id, Some(continuous()))
        .await
        .unwrap();
    wait_semantic(&mut h, |e| matches!(e, SemanticEvent::Started { .. })).await;
    feed(&h, &mut [&mut reader], 120).await;
    wait_semantic(&mut h, |e| {
        matches!(e, SemanticEvent::UtteranceFinalized { utterance: 1, .. })
    })
    .await;

    let all = semantic(&h);
    // Privacy: observations Content, everything else Operational; all in
    // the stream's session.
    for (e, class, session) in &all {
        assert_eq!(*class, e.data_class());
        assert_eq!(*session, Some(h.session.id));
    }
    // Every observation links to a transcript revision that was published,
    // and its mention is exactly the text at its span in that revision.
    let transcripts: Vec<TranscriptRevision> = h
        .events
        .events()
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::Speech(SpeechEvent::Transcript(r)) => Some((**r).clone()),
            _ => None,
        })
        .collect();
    let mut money_final = false;
    let mut resolved = false;
    for (e, _, _) in &all {
        let SemanticEvent::Observation { observation: o, .. } = e else {
            continue;
        };
        let rev = transcripts
            .iter()
            .find(|r| r.meta.utterance == o.source.utterance && r.revision == o.source.revision)
            .expect("source revision was published");
        let text = communication_engine::semantic::input_from_revision(rev).text;
        let cut: String = text
            .chars()
            .skip(o.source.span.start as usize)
            .take((o.source.span.end - o.source.span.start) as usize)
            .collect();
        match &o.payload {
            ObservationPayload::Entity(en) => {
                assert_eq!(cut, en.mention);
                if en.category == "money" && o.status == ObservationStatus::Final {
                    money_final = true;
                }
            }
            ObservationPayload::Reference(r) => {
                assert_eq!(cut, r.mention);
                if o.status == ObservationStatus::Final
                    && matches!(r.resolution, Resolution::Resolved { .. })
                {
                    assert_eq!(r.candidates[0].mention, "forty dollars");
                    assert_eq!(o.epistemic, Epistemic::ResolvedInterpretation);
                    resolved = true;
                }
            }
            _ => {}
        }
    }
    assert!(
        money_final && resolved,
        "{:?}",
        all.iter().map(|(e, _, _)| e.kind()).collect::<Vec<_>>()
    );
    // Context events never carry content.
    for (e, _, _) in &all {
        if let SemanticEvent::Context { .. } = e {
            let j = serde_json::to_string(e).unwrap();
            assert!(!j.contains("forty"), "{j}");
        }
    }
    let st = h.engine.semantic_status(id).await.unwrap();
    assert!(st.counters.processed >= 2 && st.counters.finalized >= 1);
    assert_eq!(h.engine.metrics().semantics_started, 1);
    assert_eq!(h.engine.health().await.unwrap().semantics.len(), 1);

    // Confirmation by the host.
    let money = all
        .iter()
        .find_map(|(e, _, _)| match e {
            SemanticEvent::Observation { observation, .. } if matches!(&observation.payload, ObservationPayload::Entity(en) if en.category == "money") => Some(observation.id.clone()),
            _ => None,
        })
        .unwrap();
    h.engine
        .confirm_observation(id, money.clone())
        .await
        .unwrap();
    let c = wait_semantic(&mut h, |e| {
        matches!(
            e,
            SemanticEvent::Observation {
                change: ObservationChange::Confirmed,
                ..
            }
        )
    })
    .await;
    let SemanticEvent::Observation { observation, .. } = c else {
        unreachable!()
    };
    assert_eq!(observation.id, money);
    assert_eq!(observation.epistemic, Epistemic::UserConfirmedFact);

    // Context reset.
    h.engine.reset_semantic_context(id).await.unwrap();
    wait_semantic(&mut h, |e| {
        matches!(
            e,
            SemanticEvent::Context {
                change: ContextChange::Cleared,
                ..
            }
        )
    })
    .await;

    // Recognition stop ends semantics (drained, SourceEnded).
    h.engine.stop_recognition(id).await.unwrap();
    wait_semantic(&mut h, |e| {
        matches!(
            e,
            SemanticEvent::Stopped {
                reason: SemanticEndReason::SourceEnded,
                ..
            }
        )
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let st = h.engine.semantic_status(id).await.unwrap();
    assert!(!st.running);
    assert_eq!(st.ended, Some(SemanticEndReason::SourceEnded));
    assert_eq!(st.context.items, 0);
    let report = h.engine.shutdown().await.unwrap();
    assert!(report.clean);
}

#[tokio::test]
async fn start_errors_policy_and_explicit_stop() {
    let mut config = audio_config();
    config.semantic.max_concurrent = 1;
    let remote = ScriptedSemantic::new(Script::new(vec![])).with_locality(Locality::Remote);
    let h = harness(config, two_utterances(), Some(remote)).await;
    h.grant_capture().await;
    let (_r1, a) = open(&h).await;
    let (_r2, b) = open(&h).await;
    let code = |r: Result<SemanticStatus, CeError>| r.unwrap_err().code().to_owned();
    let unknown = AudioStreamId::from_uuid(uuid::Uuid::from_u128(99));
    assert_eq!(
        code(h.engine.start_semantics(unknown, None).await),
        "audio.unknown_stream"
    );
    // The registered remote adapter is first (default) and refused by the
    // default policy.
    assert_eq!(
        code(h.engine.start_semantics(a, None).await),
        "semantic.locality_not_permitted"
    );
    let rules = SemanticConfig {
        adapter: Some("ce.semantic.rules".into()),
        ..SemanticConfig::default()
    };
    let mut bad = rules.clone();
    bad.budget.queue = 0;
    assert_eq!(
        code(h.engine.start_semantics(a, Some(bad)).await),
        "config.semantic_invalid"
    );
    let mut nope = rules.clone();
    nope.adapter = Some("nope".into());
    assert_eq!(
        code(h.engine.start_semantics(a, Some(nope)).await),
        "semantic.no_adapter"
    );
    h.engine
        .start_semantics(a, Some(rules.clone()))
        .await
        .unwrap();
    assert_eq!(
        code(h.engine.start_semantics(a, Some(rules.clone())).await),
        "semantic.already_running"
    );
    assert_eq!(
        code(h.engine.start_semantics(b, Some(rules.clone())).await),
        "semantic.capacity_exhausted"
    );
    let fin = h.engine.stop_semantics(a).await.unwrap();
    assert_eq!(fin.ended, Some(SemanticEndReason::Stopped));
    assert_eq!(
        h.engine.stop_semantics(a).await.unwrap().ended,
        Some(SemanticEndReason::Stopped)
    );
    assert_eq!(
        code(h.engine.semantic_status(b).await),
        "semantic.not_found"
    );
    assert_eq!(
        code(h.engine.reset_semantic_context(b).await),
        "semantic.not_found"
    );
    // A remote adapter is accepted only when the policy allows it.
    let mut allow = SemanticConfig {
        acts: false,
        ..SemanticConfig::default()
    };
    allow.policy.allow_remote = true;
    let st = h.engine.start_semantics(b, Some(allow)).await.unwrap();
    assert_eq!(
        st.capabilities.locality,
        Locality::Remote,
        "disclosed in status"
    );
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn streams_are_isolated_and_session_close_drains() {
    let mut h = harness(audio_config(), two_utterances(), None).await;
    h.grant_capture().await;
    let (mut ra, a) = open(&h).await;
    let (mut rb, b) = open(&h).await;
    h.engine.start_semantics(a, None).await.unwrap();
    h.engine.start_semantics(b, None).await.unwrap();
    h.engine
        .start_recognition(a, Some(continuous()))
        .await
        .unwrap();
    h.engine
        .start_recognition(b, Some(continuous()))
        .await
        .unwrap();
    feed(&h, &mut [&mut ra, &mut rb], 120).await;
    for s in [a, b] {
        wait_semantic(&mut h, |e| matches!(e, SemanticEvent::UtteranceFinalized { stream, utterance: 1, .. } if *stream == s)).await;
    }
    // Each stream resolves "it" only against its own context.
    for (e, _, _) in semantic(&h) {
        if let SemanticEvent::Observation { observation: o, .. } = e {
            if let ObservationPayload::Reference(r) = &o.payload {
                for c in &r.candidates {
                    assert!(c.item.0.starts_with("ctx."));
                }
                assert!(
                    r.candidates.len() <= 1,
                    "no candidates from the other stream"
                );
            }
        }
    }
    let id = h.session.id;
    h.engine.close_session(id).await.unwrap();
    for s in [a, b] {
        wait_semantic(&mut h, |e| matches!(e, SemanticEvent::Stopped { stream, reason: SemanticEndReason::SourceEnded, .. } if *stream == s)).await;
    }
    let stops = semantic(&h)
        .iter()
        .filter(|(e, _, _)| matches!(e, SemanticEvent::Stopped { .. }))
        .count();
    assert_eq!(stops, 2);
    h.engine.shutdown().await.unwrap();
}
