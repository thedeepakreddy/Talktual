//! Scripted sequence tests of the per-stream recognizer: partial →
//! revision → stable → final, Phase 3 hints, timestamps, gaps, format
//! changes, errors, resets, cancellation, throttling, malformed output,
//! language observations and bounded memory.

use ce_audio_intel::synth::{SYNTH_STREAM, block};
use ce_core::audio::{AudioBlock, Discontinuity, DiscontinuityKind};
use ce_core::intel::model::{DiscontinuityContext, ResultMeta};
use ce_core::intel::{
    ActivityState, AnalysisCounters, AnalysisEndReason, AnalysisEvent, DegradedMode,
    SegmentCloseReason,
};
use ce_core::speech::{
    FinalReason, HintSource, LanguageDecision, LanguageScope, RecognitionConfig,
    RecognitionEndReason, RecognitionMode, ResetReason, RevisionStatus, SegmentationMode,
    SpeechEvent, TranscriptRevision,
};
use ce_core::time::MonotonicTime;
use ce_speech::adapter::{AdapterWord, AsrFactory};
use ce_speech::fake::{ScriptedFactory, ScriptedFault, ScriptedHypothesis, UtteranceScript};
use ce_speech::recognizer::{RecognizerSetup, StreamRecognizer};
use ce_speech::transcript::TranscriptAssembler;
use ce_speech::{LanguageIdentifier, WhichlangIdentifier};
use std::sync::Arc;

fn continuous() -> RecognitionConfig {
    RecognitionConfig {
        segmentation: SegmentationMode::Continuous,
        partial_interval_ms: 50,
        ..RecognitionConfig::default()
    }
}

fn activity() -> RecognitionConfig {
    RecognitionConfig {
        partial_interval_ms: 50,
        ..RecognitionConfig::default()
    }
}

fn recognizer(
    cfg: RecognitionConfig,
    f: &dyn AsrFactory,
    analysis: bool,
    lid: bool,
) -> (StreamRecognizer, Vec<SpeechEvent>) {
    let identifier: Option<Arc<dyn LanguageIdentifier>> = if lid {
        Some(Arc::new(WhichlangIdentifier))
    } else {
        None
    };
    let setup = RecognizerSetup {
        stream: SYNTH_STREAM,
        config: cfg,
        capabilities: f.capabilities(),
        identifier,
        analysis_available: analysis,
    };
    let mut out = Vec::new();
    let mut r = StreamRecognizer::new(setup, f.create().unwrap()).unwrap();
    r.start(&mut out);
    r.load(&mut out).unwrap();
    (r, out)
}

/// `n` 10 ms silent blocks from sequence `seq0` / frame `f0`.
fn blocks(seq0: u64, f0: u64, n: u64, rate: u32, channels: u16) -> Vec<AudioBlock> {
    let frames = u64::from(rate / 100);
    (0..n)
        .map(|i| {
            block(
                seq0 + i,
                f0 + i * frames,
                rate,
                channels,
                vec![0.0; (frames * u64::from(channels)) as usize],
            )
        })
        .collect()
}

fn push_all(r: &mut StreamRecognizer, bs: &[AudioBlock], out: &mut Vec<SpeechEvent>) {
    for b in bs {
        r.push_block(b, out).unwrap();
    }
}

fn meta(f0: u64, f1: u64) -> ResultMeta {
    ResultMeta {
        stream: SYNTH_STREAM,
        sequence_range: (0, 0),
        frame_range: (f0, f1),
        time_range: (MonotonicTime::from_nanos(0), MonotonicTime::from_nanos(0)),
        discontinuity: None,
        mode: DegradedMode::Full,
    }
}

fn onset(frame: u64) -> AnalysisEvent {
    AnalysisEvent::ActivityChanged {
        meta: meta(frame, frame + 160),
        state: ActivityState::Active,
        score: None,
        decision_latency_ms: 100,
    }
}

fn segment_end(f0: u64, f1: u64) -> AnalysisEvent {
    AnalysisEvent::SegmentEnded {
        meta: meta(f0, f1),
        segment: 0,
        duration_ms: ((f1 - f0) / 16) as u32,
        reason: SegmentCloseReason::SilencePolicy,
        mean_score: None,
    }
}

fn transcripts(out: &[SpeechEvent]) -> Vec<TranscriptRevision> {
    out.iter()
        .filter_map(|e| match e {
            SpeechEvent::Transcript(r) => Some((**r).clone()),
            _ => None,
        })
        .collect()
}

fn finals(out: &[SpeechEvent]) -> Vec<TranscriptRevision> {
    transcripts(out)
        .into_iter()
        .filter(|r| r.status != RevisionStatus::Partial)
        .collect()
}

#[test]
fn partial_revision_stable_final_sequence_with_exact_timestamps() {
    let f = ScriptedFactory::new(vec![
        UtteranceScript::timed(&[
            (200, "the"),
            (400, "the cat"),
            (600, "the cap sat"),
            (800, "the cat sat"),
            (1_000, "the cat sat on"),
            (1_200, "the cat sat on the mat"),
        ])
        .endpoint_at(1_300),
    ]);
    let (mut r, mut out) = recognizer(continuous(), &f, false, false);
    assert!(matches!(out[0], SpeechEvent::Started { .. }));
    assert!(matches!(
        out[1],
        SpeechEvent::ModelReady { reused: false, .. }
    ));
    push_all(&mut r, &blocks(0, 0, 150, 16_000, 1), &mut out);
    let revs: Vec<_> = transcripts(&out)
        .into_iter()
        .filter(|r| r.meta.utterance == 0)
        .collect();
    let fin = revs.last().unwrap();
    assert_eq!(fin.status, RevisionStatus::Final);
    assert_eq!(fin.final_reason, Some(FinalReason::AdapterEndpoint));
    assert_eq!(fin.text.committed, "the cat sat on the mat");
    assert!(fin.text.provisional.is_empty());
    // Exact timeline: 1.3 s of 16 kHz audio from frame 0.
    assert_eq!(fin.meta.frame_range, (0, 20_800));
    assert_eq!(
        fin.meta.time_range.1,
        MonotonicTime::from_nanos(1_300_000_000)
    );
    // Revisions are numbered without gaps; committed text never shrinks
    // before the final; the provisional correction (cap → cat) was kept.
    let mut committed = String::new();
    let mut asm = TranscriptAssembler::new();
    for (i, r) in revs.iter().enumerate() {
        assert_eq!(r.revision as usize, i);
        if r.status == RevisionStatus::Partial {
            assert!(r.text.committed.starts_with(&committed));
            committed.clone_from(&r.text.committed);
        }
        asm.apply(r).unwrap();
        assert_eq!(asm.text(), &r.text, "deltas reproduce snapshots");
    }
    assert!(revs.iter().any(|r| r.text.full() == "the cap sat"));
    assert!(
        revs.iter()
            .any(|r| !r.text.committed.is_empty() && r.status == RevisionStatus::Partial)
    );
    // Continuous: the next utterance starts where the last one ended.
    let next = out
        .iter()
        .find_map(|e| match e {
            SpeechEvent::UtteranceStarted { meta } if meta.utterance == 1 => Some(meta.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(next.frame_range.0, 20_800);
    assert_eq!(next.hints, HintSource::Continuous);
}

#[test]
fn timestamps_map_exactly_through_resampling_and_downmix() {
    let f = ScriptedFactory::new(vec![
        UtteranceScript::progressive("one two three", 300).endpoint_at(1_000),
    ]);
    let (mut r, mut out) = recognizer(continuous(), &f, false, false);
    // 48 kHz stereo starting at frame 4800 (0.1 s) and time 0.1 s.
    push_all(&mut r, &blocks(10, 4_800, 200, 48_000, 2), &mut out);
    let fin = &finals(&out)[0];
    assert_eq!(fin.text.committed, "one two three");
    assert_eq!(fin.meta.frame_range, (4_800, 4_800 + 48_000));
    assert_eq!(
        fin.meta.time_range.0,
        MonotonicTime::from_nanos(100_000_000)
    );
    assert_eq!(
        fin.meta.time_range.1,
        MonotonicTime::from_nanos(1_100_000_000)
    );
    assert_eq!(fin.meta.sequence_range.0, 10);
}

#[test]
fn phase3_hints_open_backdated_utterances_and_finalize_after_trailing_audio() {
    let f = ScriptedFactory::new(vec![UtteranceScript::progressive("ok then", 200)]);
    let (mut r, mut out) = recognizer(activity(), &f, true, true);
    // 1 s of audio: nothing is recognized without an activity hint.
    push_all(&mut r, &blocks(0, 0, 100, 16_000, 1), &mut out);
    assert!(transcripts(&out).is_empty());
    // Onset at 0.6 s, decided later: the utterance starts at the onset,
    // served from the pre-roll ring.
    r.push_hint(&onset(9_600), &mut out).unwrap();
    let started = out.iter().find_map(|e| match e {
        SpeechEvent::UtteranceStarted { meta } => Some(meta.clone()),
        _ => None,
    });
    assert_eq!(started.unwrap().frame_range.0, 9_600);
    push_all(&mut r, &blocks(100, 16_000, 100, 16_000, 1), &mut out);
    // The segment end is about audio not yet received: the hint waits.
    r.push_hint(&segment_end(9_600, 40_000), &mut out).unwrap();
    assert_eq!(r.buffered().2, 1, "deferred until the audio arrives");
    push_all(&mut r, &blocks(200, 32_000, 100, 16_000, 1), &mut out);
    let fin = &finals(&out)[0];
    assert_eq!(fin.final_reason, Some(FinalReason::SegmentEnd));
    // Ends exactly trailing_ms (200 ms) after the segment end.
    assert_eq!(fin.meta.frame_range, (9_600, 43_200));
    assert_eq!(fin.text.committed, "ok then");
    assert_eq!(fin.meta.hints, HintSource::Activity);
    // Short text: the utterance language abstains, never guesses.
    let lang = fin.language.as_ref().unwrap();
    assert_eq!(lang.scope, LanguageScope::Utterance { utterance: 0 });
    assert!(matches!(lang.decision, LanguageDecision::Abstained { .. }));
    // No utterance opens without another onset.
    assert_eq!(r.status().open_utterance, None);
    let st = r.status();
    assert_eq!(st.counters.final_delay_ms.count, 1);
}

#[test]
fn degraded_or_stopped_analysis_falls_back_to_continuous_and_recovers() {
    let f = ScriptedFactory::new(vec![UtteranceScript::progressive("a b", 100)]);
    let (mut r, mut out) = recognizer(activity(), &f, true, false);
    push_all(&mut r, &blocks(0, 0, 10, 16_000, 1), &mut out);
    let shed = AnalysisEvent::DegradedModeChanged {
        stream: SYNTH_STREAM,
        from: DegradedMode::Reduced,
        to: DegradedMode::Shedding,
        load_permille: 900,
        backlog: 3,
    };
    r.push_hint(&shed, &mut out).unwrap();
    assert!(out.iter().any(|e| matches!(
        e,
        SpeechEvent::HintsChanged {
            hints: HintSource::Continuous,
            ..
        }
    )));
    push_all(&mut r, &blocks(10, 1_600, 10, 16_000, 1), &mut out);
    assert_eq!(r.status().open_utterance, Some(0));
    let back = AnalysisEvent::DegradedModeChanged {
        stream: SYNTH_STREAM,
        from: DegradedMode::Shedding,
        to: DegradedMode::Reduced,
        load_permille: 100,
        backlog: 0,
    };
    r.push_hint(&back, &mut out).unwrap();
    assert_eq!(r.status().hints, HintSource::Activity);
    let stop = AnalysisEvent::Stopped {
        stream: SYNTH_STREAM,
        reason: AnalysisEndReason::Stopped,
        counters: AnalysisCounters::default(),
    };
    r.push_hint(&stop, &mut out).unwrap();
    assert_eq!(r.status().hints, HintSource::Continuous);
    // Without analysis at start, ActivityHints starts in Continuous.
    let (r2, _) = recognizer(activity(), &f, false, false);
    assert_eq!(r2.status().hints, HintSource::Continuous);
}

#[test]
fn gaps_and_format_changes_finalize_and_never_span_utterances() {
    let f = ScriptedFactory::new(vec![UtteranceScript::progressive("w1 w2 w3 w4 w5", 100)]);
    let (mut r, mut out) = recognizer(continuous(), &f, false, false);
    push_all(&mut r, &blocks(0, 0, 30, 16_000, 1), &mut out);
    // Blocks 30..33 lost (tap drops show as sequence gaps).
    push_all(&mut r, &blocks(33, 5_280, 30, 16_000, 1), &mut out);
    let f0 = &finals(&out)[0];
    assert_eq!(f0.final_reason, Some(FinalReason::Discontinuity));
    assert_eq!(f0.meta.frame_range, (0, 4_800));
    assert_eq!(
        f0.meta.discontinuity,
        Some(DiscontinuityContext::AnalysisDrop { blocks: 3 })
    );
    assert!(out.iter().any(|e| matches!(
        e,
        SpeechEvent::Discontinuity {
            resume_frame: 5_280,
            ..
        }
    )));
    assert!(out.iter().any(|e| matches!(
        e,
        SpeechEvent::Reset {
            reason: ResetReason::Discontinuity,
            ..
        }
    )));
    // A flagged capture gap, then a format change (48 kHz).
    let mut flagged = blocks(63, 20_000, 1, 16_000, 1);
    flagged[0].meta.discontinuity = Some(Discontinuity {
        kind: DiscontinuityKind::DeviceOverflow,
        lost_frames: Some(10_000),
    });
    push_all(&mut r, &flagged, &mut out);
    push_all(&mut r, &blocks(64, 20_160, 20, 16_000, 1), &mut out);
    push_all(&mut r, &blocks(84, 60_000, 20, 48_000, 1), &mut out);
    r.finish(RecognitionEndReason::StreamClosed, &mut out);
    let fs = finals(&out);
    let reasons: Vec<_> = fs.iter().map(|r| r.final_reason.unwrap()).collect();
    assert_eq!(
        reasons,
        vec![
            FinalReason::Discontinuity,
            FinalReason::Discontinuity,
            FinalReason::Discontinuity,
            FinalReason::StreamEnd
        ]
    );
    assert_eq!(fs[1].meta.frame_range, (5_280, 10_080));
    assert_eq!(
        fs[3].meta.discontinuity,
        Some(DiscontinuityContext::FormatChange)
    );
    assert_eq!(fs[3].meta.frame_range.0, 60_000);
    // Utterances never overlap and numbering never repeats.
    for w in fs.windows(2) {
        assert!(w[0].meta.utterance < w[1].meta.utterance);
    }
    assert_eq!(r.status().counters.discontinuities, 3);
}

#[test]
fn adapter_errors_cancel_reset_and_repeated_errors_fail() {
    let f = ScriptedFactory::new(vec![UtteranceScript::progressive("a b c", 100)]).with_fault(
        ScriptedFault::Error {
            utterance: Some(0),
            at_ms: 250,
        },
    );
    let (mut r, mut out) = recognizer(continuous(), &f, false, false);
    push_all(&mut r, &blocks(0, 0, 60, 16_000, 1), &mut out);
    let c = &finals(&out)[0];
    assert_eq!(c.status, RevisionStatus::Cancelled);
    assert_eq!(c.final_reason, Some(FinalReason::Failure));
    assert!(out.iter().any(|e| matches!(
        e,
        SpeechEvent::Reset {
            reason: ResetReason::AdapterError,
            ..
        }
    )));
    // Recognition continues with the next utterance.
    assert_eq!(r.status().open_utterance, Some(1));
    assert_eq!(r.status().counters.adapter_errors, 1);

    let f = ScriptedFactory::new(vec![UtteranceScript::progressive("a", 100)])
        .with_fault(ScriptedFault::AlwaysError { from_utterance: 0 });
    let (mut r, mut out) = recognizer(continuous(), &f, false, false);
    let mut err = None;
    for b in blocks(0, 0, 100, 16_000, 1) {
        if let Err(e) = r.push_block(&b, &mut out) {
            err = Some(e);
            break;
        }
    }
    let e = err.expect("three consecutive errors fail the recognition");
    assert_eq!(e.code(), "speech.adapter_failed");
    r.fail(&e, &mut out);
    assert!(matches!(
        out.last(),
        Some(SpeechEvent::Stopped {
            reason: RecognitionEndReason::Failed,
            ..
        })
    ));
    // Finalize errors cancel the utterance too.
    let f = ScriptedFactory::new(vec![
        UtteranceScript::progressive("a b", 100).endpoint_at(300),
    ])
    .with_fault(ScriptedFault::FinalizeError { utterance: 0 });
    let (mut r, mut out) = recognizer(continuous(), &f, false, false);
    push_all(&mut r, &blocks(0, 0, 40, 16_000, 1), &mut out);
    assert_eq!(finals(&out)[0].status, RevisionStatus::Cancelled);
}

#[test]
fn reset_stop_stream_end_and_revocation() {
    let f = ScriptedFactory::new(vec![UtteranceScript::timed(&[
        (100, "a"),
        (200, "a b"),
        (300, "a b c"),
        (400, "a b c d"),
        (500, "a b c d e"),
    ])]);
    let (mut r, mut out) = recognizer(continuous(), &f, false, false);
    push_all(&mut r, &blocks(0, 0, 60, 16_000, 1), &mut out);
    r.reset(&mut out);
    let c = finals(&out).pop().unwrap();
    assert_eq!(c.status, RevisionStatus::Cancelled);
    assert_eq!(c.final_reason, Some(FinalReason::Reset));
    assert!(!c.text.committed.is_empty(), "committed text is kept");
    assert!(c.text.provisional.is_empty());
    // Reset audio is not fed again: the next utterance starts after it.
    push_all(&mut r, &blocks(60, 9_600, 10, 16_000, 1), &mut out);
    let next = out.iter().rev().find_map(|e| match e {
        SpeechEvent::UtteranceStarted { meta } => Some(meta.clone()),
        _ => None,
    });
    assert_eq!(next.unwrap().frame_range.0, 9_600);
    r.finish(RecognitionEndReason::Stopped, &mut out);
    let f1 = finals(&out).pop().unwrap();
    assert_eq!(
        (f1.status, f1.final_reason),
        (RevisionStatus::Final, Some(FinalReason::Stopped))
    );
    assert!(matches!(
        out.last(),
        Some(SpeechEvent::Stopped {
            reason: RecognitionEndReason::Stopped,
            ..
        })
    ));
    // Nothing after the end.
    let n = out.len();
    push_all(&mut r, &blocks(70, 11_200, 10, 16_000, 1), &mut out);
    r.finish(RecognitionEndReason::Stopped, &mut out);
    assert_eq!(out.len(), n);

    let (mut r, mut out) = recognizer(continuous(), &f, false, false);
    push_all(&mut r, &blocks(0, 0, 30, 16_000, 1), &mut out);
    r.finish(RecognitionEndReason::Revoked, &mut out);
    assert_eq!(finals(&out)[0].status, RevisionStatus::Cancelled);
}

#[test]
fn max_duration_splits_and_activity_continues() {
    let mut cfg = activity();
    cfg.finalization.max_utterance_ms = 1_000;
    let f = ScriptedFactory::new(vec![UtteranceScript::progressive("x y z", 200)]);
    let (mut r, mut out) = recognizer(cfg, &f, true, false);
    push_all(&mut r, &blocks(0, 0, 10, 16_000, 1), &mut out);
    r.push_hint(&onset(800), &mut out).unwrap();
    push_all(&mut r, &blocks(10, 1_600, 250, 16_000, 1), &mut out);
    let fs = finals(&out);
    assert!(fs.len() >= 2);
    assert_eq!(fs[0].final_reason, Some(FinalReason::MaxDuration));
    assert_eq!(fs[0].meta.frame_range, (800, 16_800));
    assert_eq!(fs[1].meta.frame_range.0, 16_800, "continuation, no gap");
}

#[test]
fn throttled_mode_suppresses_partials_but_not_finals() {
    let mut cfg = continuous();
    cfg.budget.degrade_after = 2;
    cfg.budget.recover_after = 3;
    let f = ScriptedFactory::new(vec![
        UtteranceScript::progressive("a b c d", 100).endpoint_at(900),
    ]);
    let (mut r, mut out) = recognizer(cfg, &f, false, false);
    r.note_load(2.0, 0, 0, &mut out);
    r.note_load(2.0, 0, 0, &mut out);
    assert!(out.iter().any(|e| matches!(
        e,
        SpeechEvent::ModeChanged {
            to: RecognitionMode::Throttled,
            ..
        }
    )));
    push_all(&mut r, &blocks(0, 0, 150, 16_000, 1), &mut out);
    let t = transcripts(&out);
    assert!(t.iter().all(|r| r.status != RevisionStatus::Partial));
    assert_eq!(t[0].text.committed, "a b c d");
    assert_eq!(t[0].meta.mode, RecognitionMode::Throttled);
    for _ in 0..3 {
        r.note_load(0.1, 0, 0, &mut out);
    }
    assert_eq!(r.status().mode, RecognitionMode::Full);
    // Tap drops are reported as queue pressure.
    r.note_load(0.1, 10, 7, &mut out);
    assert!(out.iter().any(|e| matches!(
        e,
        SpeechEvent::QueuePressure {
            dropped_total: 7,
            ..
        }
    )));
}

#[test]
fn malformed_adapter_output_is_sanitized_counted_and_never_invented() {
    let script = UtteranceScript {
        hypotheses: vec![
            ScriptedHypothesis {
                at_ms: 100,
                text: "bad\u{7}text".into(),
                score: Some(f32::NAN),
                words: Some(vec![AdapterWord {
                    text: "bad".into(),
                    start: 10_000_000,
                    end: 1,
                    score: None,
                }]),
                language: None,
            },
            ScriptedHypothesis {
                at_ms: 300,
                text: "good text".into(),
                ..Default::default()
            },
        ],
        endpoint_at_ms: Some(400),
        final_text: None,
    };
    let f = ScriptedFactory::new(vec![script]).with_capabilities(|c| c.word_timestamps = true);
    let (mut r, mut out) = recognizer(continuous(), &f, false, false);
    push_all(&mut r, &blocks(0, 0, 60, 16_000, 1), &mut out);
    let t = transcripts(&out);
    assert_eq!(t[0].text.full(), "badtext");
    assert!(
        t.iter().all(|r| r.score.is_none()),
        "no score semantics → no scores"
    );
    assert!(
        t[0].words.is_none(),
        "invalid timings dropped, not repaired"
    );
    // Valid synthetic timings of the declared kind are mapped exactly.
    let fin = finals(&out).pop().unwrap();
    let words = fin.words.unwrap();
    assert_eq!(words.len(), 2);
    assert_eq!(words[1].frame_range.1, 4_800);
    assert!(words.iter().all(|w| w.committed));
    assert!(r.status().counters.malformed_outputs >= 1);
}

#[test]
fn utterance_and_stream_language_are_separate_observations() {
    let f = ScriptedFactory::new(vec![
        UtteranceScript::timed(&[(300, "could you tell me where the train station is")])
            .endpoint_at(400),
    ]);
    let (mut r, mut out) = recognizer(continuous(), &f, false, true);
    push_all(&mut r, &blocks(0, 0, 130, 16_000, 1), &mut out);
    let fs = finals(&out);
    assert!(fs.len() >= 2);
    for f in &fs[..2] {
        let l = f.language.as_ref().unwrap();
        assert!(matches!(l.scope, LanguageScope::Utterance { .. }));
        assert!(
            matches!(&l.decision, LanguageDecision::Identified { language } if language.as_str() == "en")
        );
    }
    let stream: Vec<_> = out
        .iter()
        .filter_map(|e| match e {
            SpeechEvent::LanguageObserved { observation, .. } => Some(observation.clone()),
            _ => None,
        })
        .collect();
    // First: abstain (one decided utterance); then English; then no
    // repeats while the decision is unchanged.
    assert_eq!(stream.len(), 2);
    assert!(stream.iter().all(|o| o.scope == LanguageScope::Stream));
    assert!(matches!(
        stream[0].decision,
        LanguageDecision::Abstained { .. }
    ));
    assert!(
        matches!(&stream[1].decision, LanguageDecision::Identified { language } if language.as_str() == "en")
    );
    assert_eq!(
        r.status().stream_language.unwrap().decision,
        stream[1].decision
    );
}

#[test]
fn memory_stays_bounded_over_long_streams_and_hint_floods() {
    let f = ScriptedFactory::new(vec![UtteranceScript::progressive("a b c", 300)]);
    let (mut r, mut out) = recognizer(activity(), &f, true, false);
    // Ten minutes of audio with an onset every 2 s and no segment ends.
    let mut seq = 0;
    for k in 0..300u64 {
        let bs = blocks(seq, seq * 160, 200, 16_000, 1);
        seq += 200;
        push_all(&mut r, &bs, &mut out);
        r.push_hint(&onset(k * 32_000 + 100), &mut out).unwrap();
        out.clear();
        let (ring, bounds, deferred) = r.buffered();
        assert!(ring <= 16_000 * 25, "ring {ring}"); // ≤ max utterance + pre-roll
        assert!(bounds <= 64 && deferred <= 64);
    }
    // A flood of future hints is bounded and counted.
    for i in 0..1_000u64 {
        r.push_hint(&onset(1_000_000_000 + i), &mut out).unwrap();
    }
    assert!(r.buffered().2 <= 64);
    assert!(r.status().counters.hint_dropped >= 900);
}
