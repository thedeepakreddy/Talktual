//! Deterministic scripted replay through the observer (Phase 7).
//!
//! Transcript revisions are scripted with timing exactly as Phase 4
//! derives it (`anchor + frames / rate`); translation events come from the
//! **real** Phase 6 processor with a scripted adapter. Nothing here is real
//! speech or a real model: this is synthetic evidence of the
//! orchestration rules only.

use ce_core::audio::frame::frames_to_duration;
use ce_core::ids::AudioStreamId;
use ce_core::orchestration::{
    CausalKind, DeadlinePolicy, LatencyMetric, StageKind, TimelineKind, TimingUncertainty,
};
use ce_core::semantic::SemanticEvent;
use ce_core::speech::{LanguageTag, RevisionStatus, TranscriptRevision};
use ce_core::time::MonotonicTime;
use ce_core::translation::{
    PartialPolicy, SourceLanguagePolicy, TranslationConfig, TranslationEvent,
};
use ce_orchestrate::{Finding, Observer};
use ce_translate::eval::revision_for;
use ce_translate::scripted::{ScriptedFactory, scripted_capabilities};
use ce_translate::{AdapterSlot, ProcessorSetup, TranslationFactory, TranslationProcessor};

const RATE: u32 = 16_000;
const ANCHOR: u64 = 2_000_000_000;

fn stream() -> AudioStreamId {
    AudioStreamId::from_uuid(uuid::Uuid::from_u128(7))
}

fn tag(s: &str) -> LanguageTag {
    LanguageTag::parse(s).unwrap()
}

fn mt(n: u64) -> MonotonicTime {
    MonotonicTime::from_nanos(n)
}

fn at(anchor: u64, f: u64) -> MonotonicTime {
    mt(anchor + u64::try_from(frames_to_duration(f, RATE).as_nanos()).unwrap())
}

/// Utterance `u` covers [u·1 s, u·1 s + 0.5 s) on the sample clock.
fn rev(u: u64, n: u32, status: RevisionStatus, anchor: u64) -> TranscriptRevision {
    let mut r = revision_for(stream(), u, &format!("words of utterance {u} rev {n}"));
    let (f0, f1) = (u * 16_000, u * 16_000 + 8_000);
    r.meta.frame_range = (f0, f1);
    r.meta.time_range = (at(anchor, f0), at(anchor, f1));
    r.revision = n;
    r.status = status;
    r
}

fn translator() -> TranslationProcessor {
    let f = ScriptedFactory::echo(scripted_capabilities("t.echo", &[("en", "es")]));
    let c = TranslationConfig {
        target: Some(tag("es")),
        source: SourceLanguagePolicy::Explicit {
            language: tag("en"),
        },
        partials: PartialPolicy::Full,
        ..TranslationConfig::default()
    };
    let mut p = TranslationProcessor::new(ProcessorSetup {
        stream: stream(),
        adapters: vec![AdapterSlot {
            capabilities: f.capabilities(),
            adapter: f.create(&c).unwrap(),
        }],
        config: c,
        terminology: None,
    })
    .unwrap();
    p.load().unwrap();
    p
}

fn observer(deadlines: DeadlinePolicy) -> Observer {
    Observer::new(RATE, 64, deadlines, TimingUncertainty::Unknown)
}

/// Feed a transcript to the observer and the translator, then the
/// translator's events to the observer, as the runtime does. `now` is the
/// core's monotonic time at publication.
fn step(
    o: &mut Observer,
    p: &mut TranslationProcessor,
    r: &TranscriptRevision,
    now: MonotonicTime,
) -> Vec<Finding> {
    let mut f = o.on_transcript(r, now);
    let mut out = Vec::new();
    p.process(r, 0.0, &mut out).unwrap();
    for e in &out {
        f.extend(o.on_translation(e, now));
    }
    f
}

fn finalized(u: u64, revision: u32) -> SemanticEvent {
    SemanticEvent::UtteranceFinalized {
        stream: stream(),
        utterance: u,
        revision,
        observations: 0,
    }
}

#[test]
fn a_clean_stream_has_no_findings_and_measures_latency() {
    let mut o = observer(DeadlinePolicy::default());
    let mut p = translator();
    let mut started = Vec::new();
    p.start(&mut started);
    for e in &started {
        assert!(o.on_translation(e, mt(0)).is_empty());
    }
    let mut findings = Vec::new();
    for u in 0..3 {
        for n in 0..3 {
            let status = if n == 2 {
                RevisionStatus::Final
            } else {
                RevisionStatus::Partial
            };
            // Published 300 ms after the utterance's audio ended.
            let now = mt(at(ANCHOR, u * 16_000 + 8_000).as_nanos() + 300_000_000);
            findings.extend(step(&mut o, &mut p, &rev(u, n, status, ANCHOR), now));
        }
        let now = mt(at(ANCHOR, u * 16_000 + 8_000).as_nanos() + 450_000_000);
        findings.extend(o.on_semantic(&finalized(u, 2), now));
    }
    assert!(findings.is_empty(), "{findings:?}");
    let c = o.counters();
    assert_eq!(c.transcript_revisions, 9);
    assert_eq!(c.causal_violations, 0);
    assert_eq!(c.translation_runs, 1);
    assert_eq!(
        c.derived_outputs,
        9 + 3,
        "9 translation results + 3 finalizations"
    );
    let lat = o.latency();
    let get = |m| lat.iter().find(|s| s.metric == m).copied().unwrap();
    assert_eq!(get(LatencyMetric::TranscriptFinal).count, 3);
    assert!((get(LatencyMetric::TranscriptFinal).p50_ms - 300.0).abs() < 0.01);
    assert!((get(LatencyMetric::SemanticFinal).max_ms - 450.0).abs() < 0.01);
    assert_eq!(get(LatencyMetric::TranslationFinal).count, 3);
    let tl = o.timeline();
    assert_eq!(tl.segments, 1);
    assert_eq!(tl.offset_ns, Some(i64::try_from(ANCHOR).unwrap()));
}

#[test]
fn revision_contract_violations_are_reported_not_hidden() {
    let mut o = observer(DeadlinePolicy::default());
    let now = mt(ANCHOR * 10);
    assert!(
        o.on_transcript(&rev(0, 1, RevisionStatus::Partial, ANCHOR), now)
            .is_empty()
    );
    let kinds: Vec<CausalKind> = [
        rev(0, 1, RevisionStatus::Partial, ANCHOR), // duplicate
        rev(0, 0, RevisionStatus::Partial, ANCHOR), // stale
    ]
    .iter()
    .chain(std::iter::once(&rev(0, 2, RevisionStatus::Final, ANCHOR)))
    .chain(std::iter::once(&rev(0, 3, RevisionStatus::Partial, ANCHOR))) // after final
    .flat_map(|r| o.on_transcript(r, now))
    .filter_map(|f| match f {
        Finding::Causal { kind, .. } => Some(kind),
        _ => None,
    })
    .collect();
    assert_eq!(
        kinds,
        vec![
            CausalKind::DuplicateRevision,
            CausalKind::StaleRevision,
            CausalKind::AfterFinal
        ]
    );
    // A derived output referring to a revision never published.
    let f = o.on_semantic(&finalized(0, 9), now);
    assert!(matches!(
        f[..],
        [Finding::Causal {
            stage: StageKind::Semantics,
            kind: CausalKind::UnknownSource,
            ..
        }]
    ));
    assert_eq!(o.counters().causal_violations, 4);
}

#[test]
fn translation_started_resets_worker_local_sequences() {
    let mut o = observer(DeadlinePolicy::default());
    let now = mt(ANCHOR * 10);
    // First worker run.
    let mut p = translator();
    let mut ev = Vec::new();
    p.start(&mut ev);
    for n in 0..3 {
        let r = rev(0, n, RevisionStatus::Partial, ANCHOR);
        o.on_transcript(&r, now);
        p.process(&r, 0.0, &mut ev).unwrap();
    }
    let first: Vec<TranslationEvent> = std::mem::take(&mut ev);
    let mut f = Vec::new();
    for e in &first {
        f.extend(o.on_translation(e, now));
    }
    assert!(f.is_empty(), "{f:?}");
    // Replaying a result of the same run is a regression...
    let last_result = first
        .iter()
        .rev()
        .find(|e| matches!(e, TranslationEvent::Result(_)))
        .unwrap()
        .clone();
    assert!(matches!(
        o.on_translation(&last_result, now)[..],
        [Finding::Causal {
            kind: CausalKind::SequenceRegression,
            ..
        }]
    ));
    // ...but a new worker run (Started) restarts sequences at 0 legitimately.
    let mut p2 = translator();
    p2.start(&mut ev);
    let r = rev(0, 3, RevisionStatus::Final, ANCHOR);
    o.on_transcript(&r, now);
    p2.process(&r, 0.0, &mut ev).unwrap();
    let mut f = Vec::new();
    for e in &ev {
        if let TranslationEvent::Result(res) = e {
            assert_eq!(res.sequence, 0, "worker-local sequence restarted");
        }
        f.extend(o.on_translation(e, now));
    }
    assert!(f.is_empty(), "{f:?}");
    assert_eq!(o.counters().translation_runs, 2);
}

#[test]
fn timeline_reanchors_unknown_timing_out_of_order_and_gaps() {
    let mut o = observer(DeadlinePolicy::default());
    let now = mt(ANCHOR * 10);
    assert!(
        o.on_transcript(&rev(1, 0, RevisionStatus::Final, ANCHOR), now)
            .is_empty()
    );
    // A stream restart re-anchors the mapping 4 s later.
    let f = o.on_transcript(
        &rev(2, 0, RevisionStatus::Final, ANCHOR + 4_000_000_000),
        now,
    );
    assert!(f.contains(&Finding::Timeline {
        stage: StageKind::Recognition,
        kind: TimelineKind::ReAnchor,
        frame: Some(32_000)
    }));
    // Unknown timing: no anchor, no latency claimed.
    let mut unk = rev(3, 0, RevisionStatus::Final, ANCHOR);
    unk.meta.time_range = (mt(0), mt(0));
    let f = o.on_transcript(&unk, now);
    assert!(f.iter().any(|x| matches!(
        x,
        Finding::Timeline {
            kind: TimelineKind::UnknownTiming,
            ..
        }
    )));
    // A newer utterance starting earlier on the sample clock.
    let mut early = rev(4, 0, RevisionStatus::Final, ANCHOR + 4_000_000_000);
    early.meta.frame_range = (100, 900);
    early.meta.time_range = (
        at(ANCHOR + 4_000_000_000, 100),
        at(ANCHOR + 4_000_000_000, 900),
    );
    let f = o.on_transcript(&early, now);
    assert!(
        f.iter().any(|x| matches!(
            x,
            Finding::Timeline {
                kind: TimelineKind::OutOfOrder,
                ..
            }
        )),
        "{f:?}"
    );
    o.on_audio_gap(Some(1_024));
    let c = o.counters();
    assert_eq!((c.re_anchors, c.unknown_timing, c.gaps), (1, 1, 1));
    assert!(c.timeline_anomalies >= 1);
    let tl = o.timeline();
    assert_eq!(tl.segments, 2);
    assert_eq!(tl.lost_frames, 1_024);
    let lat = o.latency();
    let finals = lat
        .iter()
        .find(|s| s.metric == LatencyMetric::TranscriptFinal)
        .unwrap();
    assert_eq!(finals.count, 3, "the unknown-timing final has no latency");
}

#[test]
fn deadlines_are_typed_outcomes() {
    let mut o = observer(DeadlinePolicy {
        transcript_final_ms: Some(200),
        translation_final_ms: Some(1_000),
        semantic_final_ms: None,
    });
    let mut p = translator();
    let end = at(ANCHOR, 8_000).as_nanos();
    let f = step(
        &mut o,
        &mut p,
        &rev(0, 0, RevisionStatus::Final, ANCHOR),
        mt(end + 150_000_000),
    );
    assert!(f.is_empty(), "within budget: {f:?}");
    let end1 = at(ANCHOR, 16_000 + 8_000).as_nanos();
    let f = step(
        &mut o,
        &mut p,
        &rev(1, 0, RevisionStatus::Final, ANCHOR),
        mt(end1 + 500_000_000),
    );
    assert_eq!(
        f,
        vec![Finding::Deadline {
            stage: StageKind::Recognition,
            utterance: 1,
            latency_ms: 500,
            budget_ms: 200
        }],
        "the transcript missed; the translation (1 s budget) did not"
    );
    assert_eq!(o.counters().deadline_misses, 1);
}

#[test]
fn state_is_bounded_and_replay_is_deterministic() {
    let run = || {
        let mut o = Observer::new(
            RATE,
            32,
            DeadlinePolicy::default(),
            TimingUncertainty::Unknown,
        );
        let mut p = translator();
        let mut all = Vec::new();
        for u in 0..2_000 {
            let now = mt(at(ANCHOR, u * 16_000 + 8_000).as_nanos() + 10_000_000);
            all.extend(step(
                &mut o,
                &mut p,
                &rev(u, 0, RevisionStatus::Final, ANCHOR),
                now,
            ));
        }
        // A late derived output for a forgotten utterance is "evicted", not a violation.
        all.extend(o.on_semantic(&finalized(3, 0), mt(0)));
        assert!(o.remembered() <= 32);
        (all, o.counters().clone())
    };
    let (a, ca) = run();
    let (b, cb) = run();
    assert_eq!(a, b);
    assert_eq!(ca, cb);
    assert!(a.is_empty(), "{a:?}");
    assert_eq!(ca.evicted_references, 1);
    assert_eq!(ca.transcript_revisions, 2_000);
}

/// The **real** Phase 3 analyzer on the Phase 3 dev fixtures (clean, pink
/// noise, clipped): every timed result satisfies the timeline checks — no
/// malformed range, duration mismatch, out-of-order segment or re-anchor.
#[test]
fn the_real_analyzer_satisfies_the_timeline_checks() {
    use ce_audio_intel::eval::{self, Condition, Format, Split};
    use ce_core::intel::AnalysisConfig;
    let dev = eval::load_split(Split::Dev).unwrap();
    let cfg = AnalysisConfig::default();
    for c in [
        Condition::Clean,
        Condition::Pink(10.0),
        Condition::Clipped(20.0),
    ] {
        let s = eval::build("dev", &dev, c, Format::BASE);
        let r = eval::replay(&s, &cfg, None).unwrap();
        let mut o = Observer::new(
            Format::BASE.rate,
            64,
            DeadlinePolicy::default(),
            TimingUncertainty::Unknown,
        );
        let mut findings = Vec::new();
        for e in &r.events {
            findings.extend(o.on_analysis(e));
        }
        let timed = r
            .events
            .iter()
            .filter(|e| {
                matches!(
                    e.kind(),
                    "segment_started" | "segment_ended" | "activity_changed"
                )
            })
            .count();
        assert!(timed > 4, "{c:?}: the analyzer produced timed results");
        assert!(findings.is_empty(), "{c:?}: {findings:?}");
    }
}
