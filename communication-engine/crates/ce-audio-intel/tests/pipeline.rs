//! Analyzer-level tests: replay determinism, fixture regression gates,
//! discontinuities, formats, adaptive and resource policy, faults, stress.

use ce_audio_intel::eval::{self, Condition, Format, Split};
use ce_audio_intel::synth::{self, block, blocks, noise, sine, speechlike};
use ce_audio_intel::{ScriptedVad, StreamAnalyzer};
use ce_core::audio::{Discontinuity, DiscontinuityKind};
use ce_core::ids::AudioStreamId;
use ce_core::intel::model::DiscontinuityContext;
use ce_core::intel::{
    ActivityState, AnalysisConfig, AnalysisEndReason, AnalysisEvent, ConditionKind, DegradedMode,
    PolicyDecision, QualityReason, SegmentCloseReason, Usability, VadAlgorithm,
};

fn stream() -> AudioStreamId {
    synth::SYNTH_STREAM
}

fn run(
    cfg: &AnalysisConfig,
    vad: Option<ScriptedVad>,
    bs: &[ce_core::audio::AudioBlock],
) -> Vec<AnalysisEvent> {
    let mut a = match vad {
        Some(v) => StreamAnalyzer::with_detector(stream(), cfg.clone(), Box::new(v)).unwrap(),
        None => StreamAnalyzer::new(stream(), cfg.clone()).unwrap(),
    };
    let mut out = Vec::new();
    a.start(&mut out);
    for b in bs {
        a.push_block(b, &mut out).unwrap();
    }
    a.finish(AnalysisEndReason::StreamClosed, &mut out);
    out
}

fn count(evs: &[AnalysisEvent], kind: &str) -> usize {
    evs.iter().filter(|e| e.kind() == kind).count()
}

// ------------------------------------------------------------------ replay

#[test]
fn replay_is_bit_for_bit_deterministic() {
    let dev = eval::load_split(Split::Dev).unwrap();
    let cfg = AnalysisConfig::default();
    for c in [
        Condition::Clean,
        Condition::Pink(10.0),
        Condition::Clipped(20.0),
    ] {
        let s = eval::build("dev", &dev, c, Format::BASE);
        let a = eval::replay(&s, &cfg, None).unwrap();
        let b = eval::replay(&s, &cfg, None).unwrap();
        assert_eq!(a.events, b.events, "{c:?}");
        assert_eq!(eval::digest(&a.events), eval::digest(&b.events));
        // Every result carries the algorithm identity via Started.
        let AnalysisEvent::Started { algorithm, .. } = &a.events[0] else {
            panic!()
        };
        assert_eq!(algorithm.vad, "ce.vad.spectral");
        assert_eq!(algorithm.config_fingerprint, cfg.fingerprint());
    }
}

/// Regression gate on the held-out test split. Thresholds sit below the
/// measured values recorded in docs/phase-3-evaluation.md so that a real
/// regression fails while measurement noise does not.
#[test]
fn test_split_regression_gate() {
    let test = eval::load_split(Split::Test).unwrap();
    let cfg = AnalysisConfig::default();
    let gate = [
        (Condition::Clean, 0.95, 6),
        (Condition::White(20.0), 0.93, 5),
        (Condition::Pink(10.0), 0.88, 2),
        (Condition::Quiet(30.0), 0.93, 5),
    ];
    for (c, min_f1, min_endpoints) in gate {
        let s = eval::build("test", &test, c, Format::BASE);
        let m = eval::score(&s, &eval::replay(&s, &cfg, None).unwrap(), &cfg);
        assert!(m.f1 >= min_f1, "{c:?}: F1 {}", m.f1);
        assert!(
            m.endpoint_hits >= min_endpoints,
            "{c:?}: endpoints {}/{}",
            m.endpoint_hits,
            m.endpoints
        );
        assert!(m.false_activations_per_hour < 1.0, "{c:?}: {m:?}");
    }
    // Quality flags the conditions it should.
    let clipped = eval::build("test", &test, Condition::Clipped(20.0), Format::BASE);
    let m = eval::score(&clipped, &eval::replay(&clipped, &cfg, None).unwrap(), &cfg);
    assert!(
        m.quality_reasons.contains(&QualityReason::Clipping),
        "{m:?}"
    );
    let noisy = eval::build("test", &test, Condition::White(5.0), Format::BASE);
    let m = eval::score(&noisy, &eval::replay(&noisy, &cfg, None).unwrap(), &cfg);
    assert!(m.quality_reasons.contains(&QualityReason::LowSnr), "{m:?}");
    let quiet = eval::build("test", &test, Condition::Quiet(30.0), Format::BASE);
    let m = eval::score(&quiet, &eval::replay(&quiet, &cfg, None).unwrap(), &cfg);
    assert!(
        m.quality_reasons.contains(&QualityReason::LowLevel),
        "{m:?}"
    );
    let clean = eval::build("test", &test, Condition::Clean, Format::BASE);
    let m = eval::score(&clean, &eval::replay(&clean, &cfg, None).unwrap(), &cfg);
    assert_eq!(m.final_usability, Some(Usability::Good));
}

#[test]
fn every_supported_format_and_block_size_works() {
    let test = eval::load_split(Split::Test).unwrap();
    let cfg = AnalysisConfig::default();
    for f in [
        Format {
            rate: 8_000,
            channels: 1,
            block_frames: 80,
        },
        Format {
            rate: 22_050,
            channels: 1,
            block_frames: 333,
        },
        Format {
            rate: 44_100,
            channels: 2,
            block_frames: 441,
        },
        Format {
            rate: 48_000,
            channels: 6,
            block_frames: 1024,
        },
        Format {
            rate: 96_000,
            channels: 1,
            block_frames: 960,
        },
    ] {
        let s = eval::build("test", &test[..2], Condition::Clean, f);
        let m = eval::score(&s, &eval::replay(&s, &cfg, None).unwrap(), &cfg);
        assert!(m.f1 > 0.9, "{f:?}: {m:?}");
    }
    for hop_ms in [20, 30] {
        let cfg = AnalysisConfig {
            hop_ms,
            ..AnalysisConfig::default()
        };
        let s = eval::build("test", &test[..2], Condition::Clean, Format::BASE);
        let m = eval::score(&s, &eval::replay(&s, &cfg, None).unwrap(), &cfg);
        assert!(m.f1 > 0.9, "hop {hop_ms}: {m:?}");
    }
}

// ------------------------------------------------------------------ discontinuities

#[test]
fn discontinuities_close_segments_and_are_never_bridged() {
    let cfg = AnalysisConfig::default();
    let s = speechlike(1, 16_000, 0.3, 16_000);
    let mut bs = blocks(&s, 16_000, 1, 160);
    // A device overflow at block 50 (lost 800 frames) and a tap drop at 70.
    for b in bs.iter_mut().skip(50) {
        b.meta.frame_index += 800;
    }
    bs[50].meta.discontinuity = Some(Discontinuity {
        kind: DiscontinuityKind::DeviceOverflow,
        lost_frames: Some(800),
    });
    bs.remove(70);
    let evs = run(&cfg, Some(ScriptedVad::new(|_| 0.95)), &bs);
    let disc: Vec<_> = evs
        .iter()
        .filter_map(|e| {
            if let AnalysisEvent::Discontinuity { context, .. } = e {
                Some(*context)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        disc,
        vec![
            DiscontinuityContext::Capture {
                kind: DiscontinuityKind::DeviceOverflow,
                lost_frames: Some(800)
            },
            DiscontinuityContext::AnalysisDrop { blocks: 1 }
        ]
    );
    let reasons: Vec<_> = evs
        .iter()
        .filter_map(|e| {
            if let AnalysisEvent::SegmentEnded { reason, .. } = e {
                Some(*reason)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        reasons,
        vec![
            SegmentCloseReason::Discontinuity,
            SegmentCloseReason::Discontinuity,
            SegmentCloseReason::StreamEnd
        ]
    );
    // No segment spans a gap.
    for e in &evs {
        if let AnalysisEvent::SegmentEnded { meta, .. } = e {
            let (a, b) = meta.frame_range;
            assert!(
                !(a < 50 * 160 + 800 && b > 50 * 160 + 800),
                "segment {a}..{b} bridges the overflow"
            );
        }
    }
    // Results right after a gap name it.
    let after = evs.iter().find_map(|e| match e {
        AnalysisEvent::SegmentStarted {
            meta, segment: 1, ..
        } => Some(meta.discontinuity),
        _ => None,
    });
    assert!(matches!(
        after,
        Some(Some(DiscontinuityContext::Capture { .. }))
    ));
    // Activity/segment pairing stays balanced.
    assert_eq!(count(&evs, "segment_started"), count(&evs, "segment_ended"));
    let act: Vec<_> = evs
        .iter()
        .filter_map(|e| {
            if let AnalysisEvent::ActivityChanged { state, .. } = e {
                Some(*state)
            } else {
                None
            }
        })
        .collect();
    assert!(
        act.windows(2).all(|w| w[0] != w[1]),
        "activity alternates: {act:?}"
    );
    assert_eq!(act.last(), Some(&ActivityState::Inactive));
}

#[test]
fn format_change_and_stream_restart_reset_state() {
    let cfg = AnalysisConfig::default();
    let mut bs = blocks(&sine(300.0, 16_000, 0.3, 4_800), 16_000, 1, 160);
    let n = bs.len() as u64;
    let mut more = blocks(&sine(300.0, 48_000, 0.3, 14_400), 48_000, 2, 480);
    for (i, b) in more.iter_mut().enumerate() {
        b.meta.sequence = n + i as u64;
        b.meta.frame_index = 4_800 * 3 + i as u64 * 480;
    }
    more[10].meta.discontinuity = Some(Discontinuity {
        kind: DiscontinuityKind::StreamRestart,
        lost_frames: None,
    });
    bs.extend(more);
    let evs = run(&cfg, None, &bs);
    let disc: Vec<_> = evs
        .iter()
        .filter_map(|e| {
            if let AnalysisEvent::Discontinuity { context, .. } = e {
                Some(*context)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(disc[0], DiscontinuityContext::FormatChange);
    assert!(matches!(
        disc[1],
        DiscontinuityContext::Capture {
            kind: DiscontinuityKind::StreamRestart,
            ..
        }
    ));
}

// ------------------------------------------------------------------ quality, conditions, policy

#[test]
fn quality_conditions_and_adaptive_policy_respond_to_the_signal() {
    let cfg = AnalysisConfig::default();
    // 3 s of loud clipped tone, then 3 s of silence, then 4 s of noise at -30 dBFS.
    let mut s = sine(400.0, 16_000, 1.5, 48_000);
    synth::clip(&mut s, 1.0);
    s.extend(vec![0.0; 48_000]);
    s.extend(noise(5, 64_000, 0.05));
    let evs = run(
        &cfg,
        Some(ScriptedVad::new(|i| if i < 300 { 0.9 } else { 0.0 })),
        &blocks(&s, 16_000, 1, 160),
    );
    let on: Vec<ConditionKind> = evs
        .iter()
        .filter_map(|e| {
            if let AnalysisEvent::ConditionChanged {
                condition,
                active: true,
                ..
            } = e
            {
                Some(*condition)
            } else {
                None
            }
        })
        .collect();
    for k in [
        ConditionKind::Clipping,
        ConditionKind::Overload,
        ConditionKind::ProbableActivity,
        ConditionKind::SustainedSilence,
        ConditionKind::SustainedNoise,
    ] {
        assert!(on.contains(&k), "{k:?} not raised: {on:?}");
    }
    let pol: Vec<(PolicyDecision, bool)> = evs
        .iter()
        .filter_map(|e| {
            if let AnalysisEvent::PolicyChanged {
                decision, active, ..
            } = e
            {
                Some((*decision, *active))
            } else {
                None
            }
        })
        .collect();
    assert!(pol.contains(&(PolicyDecision::SuppressLevelEstimates, true)));
    assert!(
        pol.contains(&(PolicyDecision::SuppressLevelEstimates, false)),
        "reverted after clipping stops: {pol:?}"
    );
    assert!(
        pol.contains(&(PolicyDecision::LowConfidenceActivity, true)),
        "digital silence is unusable: {pol:?}"
    );
    assert!(
        pol.contains(&(PolicyDecision::ConservativeSegmentation, true)),
        "sustained noise: {pol:?}"
    );
    let q: Vec<_> = evs
        .iter()
        .filter_map(|e| {
            if let AnalysisEvent::QualityChanged { metrics, .. } = e {
                Some(metrics.clone())
            } else {
                None
            }
        })
        .collect();
    assert!(
        q.iter()
            .any(|m| m.reasons.contains(&QualityReason::Clipping) && m.noise_floor_dbfs.is_none())
    );
    assert!(
        q.iter()
            .any(|m| m.usability == Usability::Unusable
                && m.reasons == vec![QualityReason::NoSignal])
    );
    // Nothing here describes a person: every condition is one of the six signal indicators.
    let caps = StreamAnalyzer::new(stream(), cfg.clone())
        .unwrap()
        .capabilities();
    assert_eq!(caps.conditions.len(), 6);
    assert!(!caps.overlap_detection);
}

#[test]
fn disabling_features_disables_their_outputs() {
    let mut cfg = AnalysisConfig::default();
    cfg.quality.enabled = false;
    cfg.conditions.enabled = false;
    cfg.adaptive_policy = false;
    let evs = run(&cfg, None, &blocks(&noise(1, 64_000, 0.5), 16_000, 1, 160));
    for k in ["quality_changed", "condition_changed", "policy_changed"] {
        assert_eq!(count(&evs, k), 0, "{k}");
    }
    let caps = StreamAnalyzer::new(stream(), cfg).unwrap().capabilities();
    assert!(!caps.quality && caps.conditions.is_empty());
}

// ------------------------------------------------------------------ resource policy

#[test]
fn resource_policy_degrades_sheds_and_recovers_deterministically() {
    let mut cfg = AnalysisConfig::default();
    cfg.budget.degrade_after = 2;
    cfg.budget.recover_after = 5;
    let mut a = StreamAnalyzer::new(stream(), cfg).unwrap();
    let mut out = Vec::new();
    a.start(&mut out);
    let s = speechlike(2, 16_000, 0.3, 32_000);
    let bs = blocks(&s, 16_000, 1, 160);
    let mut modes = Vec::new();
    for (i, b) in bs.iter().enumerate() {
        a.push_block(b, &mut out).unwrap();
        let load = if (20..40).contains(&i) { 0.9 } else { 0.05 };
        a.note_load(load, 0, &mut out);
        modes.push(a.mode());
    }
    a.finish(AnalysisEndReason::Stopped, &mut out);
    let changes: Vec<(DegradedMode, DegradedMode)> = out
        .iter()
        .filter_map(|e| {
            if let AnalysisEvent::DegradedModeChanged { from, to, .. } = e {
                Some((*from, *to))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        changes,
        vec![
            (DegradedMode::Full, DegradedMode::Reduced),
            (DegradedMode::Reduced, DegradedMode::Shedding),
            (DegradedMode::Shedding, DegradedMode::Reduced),
            (DegradedMode::Reduced, DegradedMode::Full)
        ]
    );
    assert!(out.iter().any(|e| matches!(
        e,
        AnalysisEvent::SegmentEnded {
            reason: SegmentCloseReason::ResourcePolicy,
            ..
        }
    )));
    let c = a.counters();
    assert!(c.shed_blocks > 0 && c.max_load_permille == 900);
    // Results produced while degraded say so.
    assert!(out.iter().any(|e| matches!(e, AnalysisEvent::SegmentStarted { meta, .. } if meta.mode == DegradedMode::Reduced)) || out.iter().any(|e| matches!(e, AnalysisEvent::ActivityChanged { meta, .. } if meta.mode != DegradedMode::Full)));
    assert_eq!(*modes.last().unwrap(), DegradedMode::Full);
}

// ------------------------------------------------------------------ faults

#[test]
fn malformed_and_hostile_input_never_panics() {
    let cfg = AnalysisConfig::default();
    let mut a = StreamAnalyzer::new(stream(), cfg).unwrap();
    let mut out = Vec::new();
    a.start(&mut out);
    let mut seq = 0;
    let mut frame = 0;
    let mut push = |a: &mut StreamAnalyzer,
                    samples: Vec<f32>,
                    ch: u16,
                    rate: u32,
                    out: &mut Vec<AnalysisEvent>| {
        let frames = samples.len() as u64 / u64::from(ch.max(1));
        let b = block(seq, frame, rate, ch, samples);
        seq += 1;
        frame += frames;
        a.push_block(&b, out)
    };
    assert!(push(&mut a, vec![f32::NAN; 160], 1, 16_000, &mut out).is_ok());
    assert!(push(&mut a, vec![f32::INFINITY; 160], 1, 16_000, &mut out).is_ok());
    assert!(push(&mut a, vec![1e30; 160], 1, 16_000, &mut out).is_ok());
    assert!(push(&mut a, vec![], 1, 16_000, &mut out).is_ok());
    assert!(
        push(&mut a, vec![0.0; 7], 2, 16_000, &mut out).is_ok(),
        "partial frame ignored"
    );
    let err = push(&mut a, vec![0.0; 160], 1, 200_000, &mut out).unwrap_err();
    assert_eq!(err.code(), "analysis.unsupported_format");
    let mut a2 = StreamAnalyzer::new(stream(), AnalysisConfig::default()).unwrap();
    let mut b = block(0, 0, 16_000, 1, vec![0.1; 160]);
    b.meta.frames = 1_000; // claims more frames than it carries
    assert!(a2.push_block(&b, &mut out).is_ok());
    assert!(a.counters().non_finite_samples >= 320);
    // Results after finish are ignored; finish is idempotent.
    a.finish(AnalysisEndReason::Stopped, &mut out);
    let n = out.len();
    a.finish(AnalysisEndReason::Stopped, &mut out);
    assert!(push(&mut a, vec![0.5; 160], 1, 16_000, &mut out).is_ok());
    assert_eq!(out.len(), n);
}

#[test]
fn energy_detector_is_a_drop_in_alternative() {
    let cfg = AnalysisConfig {
        vad: VadAlgorithm::Energy,
        ..AnalysisConfig::default()
    };
    let test = eval::load_split(Split::Test).unwrap();
    let s = eval::build("test", &test, Condition::Clean, Format::BASE);
    let r = eval::replay(&s, &cfg, None).unwrap();
    let AnalysisEvent::Started { algorithm, .. } = &r.events[0] else {
        panic!()
    };
    assert_eq!(algorithm.vad, "ce.vad.energy");
    assert!(eval::score(&s, &r, &cfg).f1 > 0.95);
}

// ------------------------------------------------------------------ stress

#[test]
fn long_streams_stay_bounded_and_fast() {
    // Minutes of alternating speech-like bursts and noise at 48 kHz stereo
    // (10 in optimized builds, 3 in debug builds).
    let minutes: u32 = if cfg!(debug_assertions) { 3 } else { 10 };
    let cfg = AnalysisConfig::default();
    let mut a = StreamAnalyzer::new(stream(), cfg).unwrap();
    let mut out = Vec::new();
    a.start(&mut out);
    let unit = {
        let mut u = speechlike(3, 48_000, 0.3, 96_000);
        u.extend(noise(4, 48_000, 0.01));
        synth::to_channels(&u, 2, &[1.0, 0.8])
    };
    let t0 = std::time::Instant::now();
    let mut seq = 0u64;
    let mut frame = 0u64;
    let mut max_events_per_minute = 0usize;
    for minute in 0..minutes {
        let before = out.len();
        for _ in 0..20 {
            for c in unit.chunks(480 * 2) {
                a.push_block(&block(seq, frame, 48_000, 2, c.to_vec()), &mut out)
                    .unwrap();
                seq += 1;
                frame += 480;
            }
        }
        max_events_per_minute = max_events_per_minute.max(out.len() - before);
        let _ = minute;
        out.shrink_to(0); // the consumer would drain; keep memory flat here
        out.clear();
    }
    let rtf = t0.elapsed().as_secs_f64() / (60.0 * f64::from(minutes));
    let bound = if cfg!(debug_assertions) { 0.5 } else { 0.05 };
    assert!(rtf < bound, "real-time factor {rtf}");
    assert!(
        max_events_per_minute < 400,
        "events stay sparse: {max_events_per_minute}/min"
    );
    assert_eq!(a.counters().hops, 6_000 * u64::from(minutes));
}
