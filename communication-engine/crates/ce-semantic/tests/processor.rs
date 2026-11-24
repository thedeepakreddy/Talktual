//! Processor gates: revisions, stale input, identity, lifecycle, context,
//! references, contract enforcement, limits, policy. Deterministic; no
//! network, no model.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use ce_core::ids::AudioStreamId;
use ce_core::semantic::*;
use ce_core::speech::RevisionStatus;
use ce_core::time::MonotonicTime;
use ce_semantic::scripted::{Script, ScriptedFactory, Step, words};
use ce_semantic::{
    Draft, DraftPayload, ProcessorSetup, RulesFactory, SemanticFactory, SemanticProcessor,
};

fn stream(n: u128) -> AudioStreamId {
    AudioStreamId::from_uuid(uuid::Uuid::from_u128(n))
}

fn input(
    s: AudioStreamId,
    utt: u64,
    rev: u32,
    status: RevisionStatus,
    committed: &str,
    provisional: &str,
    at_ms: u64,
) -> SemanticInput {
    let text = match (committed.is_empty(), provisional.is_empty()) {
        (_, true) => committed.to_string(),
        (true, false) => provisional.to_string(),
        _ => format!("{committed} {provisional}"),
    };
    SemanticInput {
        stream: s,
        utterance: utt,
        revision: rev,
        status,
        committed_chars: committed.chars().count() as u32,
        text,
        language: None,
        recognition_score: None,
        time_range: (
            MonotonicTime::from_nanos(at_ms.saturating_sub(1_000) * 1_000_000),
            MonotonicTime::from_nanos(at_ms * 1_000_000),
        ),
        audio_ms: 1_000,
    }
}

fn final_(s: AudioStreamId, utt: u64, rev: u32, text: &str, at_ms: u64) -> SemanticInput {
    input(s, utt, rev, RevisionStatus::Final, text, "", at_ms)
}

fn rules(config: SemanticConfig, s: AudioStreamId) -> SemanticProcessor {
    let f = RulesFactory;
    let adapter = f.create(&config).unwrap();
    let mut p = SemanticProcessor::new(
        ProcessorSetup {
            stream: s,
            config,
            capabilities: f.capabilities(),
        },
        adapter,
    )
    .unwrap();
    p.start(&mut Vec::new());
    p
}

fn scripted(config: SemanticConfig, steps: Vec<Step>) -> (SemanticProcessor, Arc<Mutex<Script>>) {
    let script = Script::new(steps);
    let f = ScriptedFactory::new(script.clone());
    let adapter = f.create(&config).unwrap();
    let mut p = SemanticProcessor::new(
        ProcessorSetup {
            stream: stream(1),
            config,
            capabilities: f.capabilities(),
        },
        adapter,
    )
    .unwrap();
    p.start(&mut Vec::new());
    (p, script)
}

fn run(p: &mut SemanticProcessor, i: SemanticInput) -> Vec<SemanticEvent> {
    let mut out = Vec::new();
    p.process(i, 0.0, &mut out).unwrap();
    out
}

fn changes(ev: &[SemanticEvent]) -> Vec<(ObservationChange, String, String)> {
    ev.iter()
        .filter_map(|e| match e {
            SemanticEvent::Observation {
                change,
                observation,
            } => Some((
                *change,
                observation.id.0.clone(),
                match &observation.payload {
                    ObservationPayload::Entity(e) => format!("{}={}", e.category, e.mention),
                    ObservationPayload::Reference(r) => format!("ref={}", r.mention),
                    ObservationPayload::Act(_) => "act".into(),
                    ObservationPayload::Relation(r) => format!("rel={}", r.kind),
                },
            )),
            _ => None,
        })
        .collect()
}

fn observations(ev: &[SemanticEvent]) -> Vec<SemanticObservation> {
    ev.iter()
        .filter_map(|e| match e {
            SemanticEvent::Observation { observation, .. } => Some((**observation).clone()),
            _ => None,
        })
        .collect()
}

fn rejected(ev: &[SemanticEvent]) -> Vec<RejectReason> {
    ev.iter()
        .filter_map(|e| match e {
            SemanticEvent::InputRejected { reason, .. } => Some(*reason),
            _ => None,
        })
        .collect()
}

fn no_acts() -> SemanticConfig {
    SemanticConfig {
        acts: false,
        ..SemanticConfig::default()
    }
}

#[test]
fn observations_update_stabilize_finalize_and_retract_with_stable_ids() {
    let s = stream(1);
    let mut p = rules(no_acts(), s);
    // rev 0: provisional "TWENTY"
    let e0 = run(
        &mut p,
        input(s, 1, 0, RevisionStatus::Partial, "", "WAIT TWENTY", 500),
    );
    let c0 = changes(&e0);
    assert_eq!(c0.len(), 1);
    assert_eq!(c0[0].0, ObservationChange::New);
    assert_eq!(c0[0].2, "number=TWENTY");
    let id = c0[0].1.clone();
    let o = &observations(&e0)[0];
    assert_eq!(o.status, ObservationStatus::Provisional);
    assert_eq!(o.source.revision, 0);
    assert_eq!(o.source.span, TextSpan { start: 5, end: 11 });
    assert!(!o.source.committed);
    assert_eq!(o.provenance.adapter, "ce.semantic.rules");
    assert_eq!(o.provenance.locality, Locality::InProcess);

    // rev 1: "TWENTY FIVE MINUTES" provisional → same id, updated to a duration?
    // Category changes (number → duration), so a new observation replaces it.
    let e1 = run(
        &mut p,
        input(s, 1, 1, RevisionStatus::Partial, "WAIT", "TWENTY FIVE", 900),
    );
    let c1 = changes(&e1);
    assert_eq!(c1.len(), 1);
    assert_eq!(
        c1[0],
        (
            ObservationChange::Updated,
            id.clone(),
            "number=TWENTY FIVE".into()
        )
    );
    assert_eq!(observations(&e1)[0].version, 2);

    // rev 2: the number is committed → Stabilized.
    let e2 = run(
        &mut p,
        input(
            s,
            1,
            2,
            RevisionStatus::Partial,
            "WAIT TWENTY FIVE",
            "",
            1_200,
        ),
    );
    assert_eq!(
        changes(&e2),
        vec![(
            ObservationChange::Stabilized,
            id.clone(),
            "number=TWENTY FIVE".into()
        )]
    );
    assert!(e2.iter().any(|e| matches!(
        e,
        SemanticEvent::Context {
            change: ContextChange::Added,
            ..
        }
    )));

    // rev 3: unchanged → no event.
    let e3 = run(
        &mut p,
        input(
            s,
            1,
            3,
            RevisionStatus::Partial,
            "WAIT TWENTY FIVE",
            "PLEASE",
            1_400,
        ),
    );
    assert!(changes(&e3).is_empty());

    // final amends the committed text: the number is gone → Retracted,
    // its context item invalidated; the new duration is Final.
    let e4 = run(&mut p, final_(s, 1, 4, "WAIT FOR FIVE MINUTES", 1_800));
    let c4 = changes(&e4);
    assert!(
        c4.contains(&(
            ObservationChange::Retracted,
            id.clone(),
            "number=TWENTY FIVE".into()
        )),
        "{c4:?}"
    );
    assert!(
        c4.iter()
            .any(|(c, _, m)| *c == ObservationChange::New && m == "duration=FIVE MINUTES")
    );
    assert!(e4.iter().any(|e| matches!(
        e,
        SemanticEvent::Context {
            change: ContextChange::Invalidated,
            ..
        }
    )));
    assert!(e4.iter().any(|e| matches!(
        e,
        SemanticEvent::UtteranceFinalized {
            utterance: 1,
            revision: 4,
            observations: 1,
            ..
        }
    )));
    let fin = observations(&e4)
        .into_iter()
        .find(|o| o.status == ObservationStatus::Final)
        .unwrap();
    assert!(fin.source.committed);
    assert_eq!(fin.source.revision_status, RevisionStatus::Final);

    let c = p.counters();
    assert_eq!(
        (c.observations_new, c.updated, c.stabilized, c.retracted),
        (2, 1, 1, 1)
    );
}

#[test]
fn stale_duplicate_and_late_inputs_never_become_current() {
    let s = stream(1);
    let mut p = rules(no_acts(), s);
    run(
        &mut p,
        input(s, 1, 3, RevisionStatus::Partial, "", "TWO", 500),
    );
    let e = run(
        &mut p,
        input(s, 1, 2, RevisionStatus::Partial, "", "NINE", 400),
    );
    assert_eq!(rejected(&e), vec![RejectReason::Stale]);
    assert!(changes(&e).is_empty());
    let e = run(
        &mut p,
        input(s, 1, 3, RevisionStatus::Partial, "", "TWO", 500),
    );
    assert_eq!(rejected(&e), vec![RejectReason::Duplicate]);
    run(&mut p, final_(s, 1, 4, "TWO", 600));
    let e = run(
        &mut p,
        input(s, 1, 5, RevisionStatus::Partial, "", "THREE", 700),
    );
    assert_eq!(rejected(&e), vec![RejectReason::AfterFinal]);
    // A late final for an earlier revision number of a closed utterance too.
    let e = run(&mut p, final_(s, 1, 1, "SEVEN", 300));
    assert_eq!(rejected(&e), vec![RejectReason::AfterFinal]);
    // Current observations: only the finalized "TWO".
    let cur: Vec<_> = p.current().into_iter().map(|o| o.status).collect();
    assert_eq!(cur, vec![ObservationStatus::Final]);
    assert_eq!(p.counters().stale_rejected, 4);
    // Revision gaps are fine (coalescing): a new utterance can start at 7.
    let e = run(
        &mut p,
        input(s, 2, 7, RevisionStatus::Partial, "", "SIX", 900),
    );
    assert_eq!(changes(&e).len(), 1);
}

#[test]
fn cancellation_retracts_everything_and_invalidates_context() {
    let s = stream(1);
    let mut p = rules(no_acts(), s);
    run(
        &mut p,
        input(
            s,
            1,
            0,
            RevisionStatus::Partial,
            "TEN DOLLARS",
            "MAYBE",
            500,
        ),
    );
    assert_eq!(p.context().size().items, 1);
    let e = run(
        &mut p,
        input(s, 1, 1, RevisionStatus::Cancelled, "TEN DOLLARS", "", 600),
    );
    let c = changes(&e);
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].0, ObservationChange::Retracted);
    assert!(
        e.iter()
            .any(|e| matches!(e, SemanticEvent::UtteranceCancelled { utterance: 1, .. }))
    );
    assert_eq!(p.context().size().items, 0);
    assert!(p.current().is_empty());
    let e = run(&mut p, final_(s, 1, 2, "TEN DOLLARS", 700));
    assert_eq!(rejected(&e), vec![RejectReason::AfterFinal]);
}

#[test]
fn references_resolve_only_to_permitted_compatible_context() {
    let s = stream(1);
    let mut cfg = no_acts();
    cfg.context.max_utterance_distance = 3;
    let mut p = rules(cfg, s);
    run(&mut p, final_(s, 1, 0, "THE TOTAL IS FORTY DOLLARS", 2_000));
    let e = run(&mut p, final_(s, 2, 0, "CAN YOU SPLIT IT", 4_000));
    let r = observations(&e)
        .into_iter()
        .find_map(|o| match o.payload {
            ObservationPayload::Reference(r) => {
                Some((r, o.epistemic, o.uncertainty.level, o.evidence))
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(r.0.mention, "IT");
    let Resolution::Resolved { target } = &r.0.resolution else {
        panic!("{:?}", r.0.resolution)
    };
    assert_eq!(r.0.candidates.len(), 1);
    assert_eq!(r.0.candidates[0].mention, "FORTY DOLLARS");
    assert_eq!(
        r.1,
        Epistemic::ResolvedInterpretation,
        "a resolution is not an observation"
    );
    assert_eq!(r.2, Certainty::Medium);
    assert!(
        r.3.iter()
            .any(|e| e.kind == EvidenceKind::Context && e.context_item.as_ref() == Some(target))
    );
    // The refers_to relation links the reference to the context item.
    assert!(changes(&e).iter().any(|c| c.2 == "rel=refers_to"));

    // Two compatible candidates → ambiguous, no forced choice.
    run(&mut p, final_(s, 3, 0, "AND TWELVE EUROS", 5_000));
    let e = run(&mut p, final_(s, 4, 0, "PAY IT", 6_000));
    let r = observations(&e)
        .into_iter()
        .find_map(|o| match o.payload {
            ObservationPayload::Reference(r) => Some((r, o.epistemic)),
            _ => None,
        })
        .unwrap();
    assert_eq!(r.0.resolution, Resolution::Ambiguous);
    assert_eq!(r.0.candidates.len(), 2);
    assert_eq!(
        r.0.candidates[0].mention, "TWELVE EUROS",
        "most recent first"
    );
    assert_eq!(r.1, Epistemic::Hypothesis);
}

#[test]
fn context_is_bounded_by_distance_ttl_items_and_bytes() {
    let s = stream(1);
    // Utterance distance: only the previous utterance is a candidate.
    let mut cfg = no_acts();
    cfg.context.max_utterance_distance = 1;
    let mut p = rules(cfg, s);
    run(&mut p, final_(s, 1, 0, "IT IS TEN DOLLARS", 1_000));
    run(&mut p, final_(s, 2, 0, "OKAY GOOD", 2_000));
    let e = run(&mut p, final_(s, 3, 0, "PAY IT", 3_000));
    let unresolved = observations(&e).iter().any(|o| matches!(&o.payload, ObservationPayload::Reference(r) if matches!(r.resolution, Resolution::Unresolved { .. })));
    assert!(unresolved, "two utterances back is outside the distance");

    // TTL on the stream timeline.
    let mut cfg = no_acts();
    cfg.context.ttl_ms = 5_000;
    let mut p = rules(cfg, s);
    run(&mut p, final_(s, 1, 0, "TEN DOLLARS", 1_000));
    assert_eq!(p.context().size().items, 1);
    let e = run(&mut p, final_(s, 2, 0, "HMM", 7_000));
    assert!(e.iter().any(|e| matches!(
        e,
        SemanticEvent::Context {
            change: ContextChange::Expired,
            ..
        }
    )));
    assert_eq!(p.context().size().items, 0);
    assert_eq!(p.counters().context_expired, 1);

    // Item limit: oldest evicted.
    let mut cfg = no_acts();
    cfg.context.max_items = 2;
    let mut p = rules(cfg, s);
    run(
        &mut p,
        final_(s, 1, 0, "ONE DOLLAR TWO DOLLARS THREE DOLLARS", 1_000),
    );
    assert_eq!(p.context().size().items, 2);
    let kept: Vec<_> = p.context().items().map(|i| i.mention.clone()).collect();
    assert_eq!(kept, vec!["TWO DOLLARS", "THREE DOLLARS"]);
    assert_eq!(p.counters().context_evicted, 1);

    // Byte limit.
    let mut cfg = no_acts();
    cfg.context.max_bytes = 256;
    let mut p = rules(cfg, s);
    run(
        &mut p,
        final_(
            s,
            1,
            0,
            "ONE DOLLAR TWO DOLLARS THREE DOLLARS FOUR DOLLARS",
            1_000,
        ),
    );
    assert!(p.context().size().bytes <= 256);

    // Weak hypotheses never become context; provisional text never does.
    let mut p = rules(no_acts(), s);
    run(&mut p, final_(s, 1, 0, "A HUNDRED POUNDS", 1_000));
    assert_eq!(
        p.context().size().items,
        0,
        "Low certainty stays out of context"
    );
    run(
        &mut p,
        input(s, 2, 0, RevisionStatus::Partial, "", "TEN DOLLARS", 2_000),
    );
    assert_eq!(p.context().size().items, 0);
}

#[test]
fn context_scopes_off_and_utterance() {
    let s = stream(1);
    let mut cfg = no_acts();
    cfg.context.scope = ContextScopeMode::Off;
    let mut p = rules(cfg, s);
    run(&mut p, final_(s, 1, 0, "TEN DOLLARS", 1_000));
    let e = run(&mut p, final_(s, 2, 0, "PAY IT", 2_000));
    assert!(observations(&e).iter().any(|o| matches!(&o.payload, ObservationPayload::Reference(r) if r.resolution == Resolution::Unresolved { reason: "context_disabled".into() })));
    assert_eq!(p.context().size().items, 0);

    let mut cfg = no_acts();
    cfg.context.scope = ContextScopeMode::Utterance;
    let mut p = rules(cfg, s);
    run(
        &mut p,
        input(s, 1, 0, RevisionStatus::Partial, "TEN DOLLARS", "", 1_000),
    );
    assert_eq!(p.context().size().items, 1);
    let e = run(
        &mut p,
        input(
            s,
            1,
            1,
            RevisionStatus::Partial,
            "TEN DOLLARS PAY IT",
            "",
            1_500,
        ),
    );
    assert!(observations(&e).iter().any(|o| matches!(&o.payload, ObservationPayload::Reference(r) if matches!(r.resolution, Resolution::Resolved { .. }))), "same-utterance committed context");
    run(&mut p, final_(s, 1, 2, "TEN DOLLARS PAY IT", 2_000));
    assert_eq!(
        p.context().size().items,
        0,
        "utterance scope ends with the utterance"
    );
}

#[test]
fn a_correction_invalidates_context_and_dependent_resolutions() {
    let s = stream(1);
    let mut p = rules(no_acts(), s);
    // Utterance 1 has committed money; utterance 2 refers to it (final).
    run(
        &mut p,
        input(
            s,
            1,
            0,
            RevisionStatus::Partial,
            "IT IS FIFTY DOLLARS",
            "",
            1_000,
        ),
    );
    let e = run(&mut p, final_(s, 2, 0, "SPLIT IT", 2_000));
    assert!(observations(&e).iter().any(|o| matches!(&o.payload, ObservationPayload::Reference(r) if matches!(r.resolution, Resolution::Resolved { .. }))));
    // Utterance 1's final amends the committed text: the money changes.
    let e = run(&mut p, final_(s, 1, 1, "IT IS FIFTEEN DOLLARS", 2_500));
    let c = changes(&e);
    assert!(
        c.iter()
            .any(|x| x.0 == ObservationChange::Finalized && x.2 == "money=FIFTEEN DOLLARS"),
        "{c:?}"
    );
    assert!(
        c.iter()
            .any(|x| x.0 == ObservationChange::Retracted && x.2 == "ref=IT"),
        "the dependent resolution is retracted: {c:?}"
    );
    assert!(
        c.iter()
            .any(|x| x.0 == ObservationChange::Retracted && x.2 == "rel=refers_to")
    );
    assert!(e.iter().any(|e| matches!(
        e,
        SemanticEvent::Context {
            change: ContextChange::Invalidated,
            ..
        }
    )));
}

#[test]
fn contexts_are_isolated_between_streams_and_cleared_on_reset_and_stop() {
    let (a, b) = (stream(1), stream(2));
    let mut pa = rules(no_acts(), a);
    let mut pb = rules(no_acts(), b);
    run(&mut pa, final_(a, 1, 0, "TEN DOLLARS", 1_000));
    let e = run(&mut pb, final_(b, 1, 0, "PAY IT", 2_000));
    assert!(
        observations(&e).iter().any(
            |o| matches!(&o.payload, ObservationPayload::Reference(r) if r.candidates.is_empty())
        ),
        "no cross-stream leakage"
    );
    assert!(observations(&e).iter().all(|o| o.source.stream == b));

    let mut out = Vec::new();
    pa.reset_context(&mut out);
    assert!(matches!(
        out.as_slice(),
        [SemanticEvent::Context {
            change: ContextChange::Cleared,
            size: ContextSize { items: 0, bytes: 0 },
            ..
        }]
    ));
    run(&mut pa, final_(a, 2, 0, "TEN DOLLARS", 3_000));
    run(
        &mut pa,
        input(a, 3, 0, RevisionStatus::Partial, "", "AND", 3_500),
    );
    let mut out = Vec::new();
    pa.finish(SemanticEndReason::Stopped, &mut out);
    assert!(
        out.iter()
            .any(|e| matches!(e, SemanticEvent::UtteranceCancelled { utterance: 3, .. })),
        "open utterances are cancelled at stop"
    );
    assert!(out.iter().any(|e| matches!(
        e,
        SemanticEvent::Context {
            change: ContextChange::Cleared,
            ..
        }
    )));
    assert!(matches!(
        out.last(),
        Some(SemanticEvent::Stopped {
            reason: SemanticEndReason::Stopped,
            ..
        })
    ));
    assert_eq!(pa.context().size().items, 0);
    assert!(!pa.status().running);
}

#[test]
fn invalid_inputs_are_rejected_and_finals_still_close() {
    let s = stream(1);
    let mut cfg = no_acts();
    cfg.budget.max_input_chars = 20;
    let mut p = rules(cfg, s);
    run(
        &mut p,
        input(s, 1, 0, RevisionStatus::Partial, "", "TEN", 100),
    );
    let e = run(
        &mut p,
        final_(s, 1, 1, "TEN AND A VERY LONG TAIL OF WORDS", 200),
    );
    assert_eq!(rejected(&e), vec![RejectReason::Oversized]);
    assert!(
        changes(&e)
            .iter()
            .all(|c| c.0 == ObservationChange::Retracted)
    );
    assert!(e.iter().any(|e| matches!(
        e,
        SemanticEvent::UtteranceFinalized {
            observations: 0,
            ..
        }
    )));
    let mut bad = final_(s, 2, 0, "TEN", 300);
    bad.committed_chars = 99;
    assert_eq!(rejected(&run(&mut p, bad)), vec![RejectReason::Malformed]);
    let bad = final_(s, 3, 0, "TEN\u{7}", 300);
    assert_eq!(rejected(&run(&mut p, bad)), vec![RejectReason::Malformed]);
    assert_eq!(p.counters().invalid_rejected, 3);
}

fn draft(text: &str, start: u32, end: u32) -> Draft {
    let mut d = words(text).remove(0);
    d.span = TextSpan { start, end };
    d
}

#[test]
fn contract_violations_by_adapters_are_dropped() {
    let text = "ALPHA BETA";
    let ok = words(text).remove(0);
    let mut out_of_range = ok.clone();
    out_of_range.span = TextSpan { start: 0, end: 50 };
    let wrong_mention = draft(text, 6, 10); // mention "ALPHA" at "BETA"
    let mut confirmed = ok.clone();
    confirmed.epistemic = Epistemic::UserConfirmedFact;
    let mut no_evidence = ok.clone();
    no_evidence.evidence.clear();
    let mut calibrated = ok.clone();
    calibrated.uncertainty.calibrated = true;
    let mut scored = ok.clone();
    scored.uncertainty.score = Some(SemanticScore {
        value: f32::NAN,
        semantics: ce_core::intel::ScoreSemantics::HeuristicConfidence,
    });
    let dangling = Draft {
        key: "relation:x".into(),
        payload: DraftPayload::Relation {
            kind: "x".into(),
            from: 1,
            to: ce_semantic::RelationTarget::Draft(0),
        },
        ..ok.clone()
    };
    let (mut p, _) = scripted(
        no_acts(),
        vec![Step::Output(vec![
            ok,
            out_of_range,
            wrong_mention,
            confirmed,
            no_evidence,
            calibrated,
            scored,
            dangling,
        ])],
    );
    let e = run(&mut p, final_(stream(1), 1, 0, text, 100));
    assert_eq!(changes(&e).len(), 1, "only the valid draft survives");
    assert_eq!(p.counters().invalid_drafts, 7);
    assert!(e.iter().any(|e| matches!(
        e,
        SemanticEvent::Degraded {
            reason: DegradeReason::InvalidOutput,
            ..
        }
    )));
    // The transcript text is never edited: every mention equals its span.
    for o in observations(&e) {
        let ObservationPayload::Entity(en) = &o.payload else {
            continue;
        };
        let cut: String = text
            .chars()
            .skip(o.source.span.start as usize)
            .take((o.source.span.end - o.source.span.start) as usize)
            .collect();
        assert_eq!(cut, en.mention);
    }
}

#[test]
fn output_limits_min_certainty_and_partials_policy() {
    let mut cfg = no_acts();
    cfg.budget.max_observations = 2;
    let (mut p, _) = scripted(cfg, vec![]);
    let e = run(&mut p, final_(stream(1), 1, 0, "A B C D", 100));
    assert_eq!(changes(&e).len(), 2);
    assert!(e.iter().any(|e| matches!(
        e,
        SemanticEvent::Degraded {
            reason: DegradeReason::OutputLimited,
            ..
        }
    )));
    assert_eq!(p.counters().output_truncated, 1);

    let s = stream(1);
    let mut cfg = no_acts();
    cfg.min_certainty = Certainty::Medium;
    let mut p = rules(cfg, s);
    let e = run(
        &mut p,
        final_(s, 1, 0, "A HUNDRED POUNDS AND TEN DOLLARS", 100),
    );
    let c = changes(&e);
    assert_eq!(c.len(), 1, "{c:?}");
    assert_eq!(c[0].2, "money=TEN DOLLARS");
    assert_eq!(p.counters().below_min_certainty, 1);

    let mut cfg = no_acts();
    cfg.budget.partials = false;
    let (mut p, script) = scripted(cfg, vec![]);
    run(
        &mut p,
        input(stream(1), 1, 0, RevisionStatus::Partial, "", "A", 100),
    );
    run(&mut p, final_(stream(1), 1, 1, "A B", 200));
    assert_eq!(
        script.lock().unwrap().calls.len(),
        1,
        "partials not interpreted"
    );
}

#[test]
fn adapter_errors_deadlines_and_repeated_failure() {
    let mut cfg = no_acts();
    cfg.budget.max_consecutive_errors = 2;
    cfg.budget.deadline_ms = 5;
    let (mut p, _) = scripted(
        cfg,
        vec![
            Step::Words,
            Step::Error,
            Step::Sleep(Duration::from_millis(30)),
            Step::Error,
            Step::Error,
        ],
    );
    let s = stream(1);
    run(
        &mut p,
        input(s, 1, 0, RevisionStatus::Partial, "", "A", 100),
    );
    let e = run(
        &mut p,
        input(s, 1, 1, RevisionStatus::Partial, "", "A B", 200),
    );
    assert!(e.iter().any(|e| matches!(
        e,
        SemanticEvent::Degraded {
            reason: DegradeReason::AdapterError,
            ..
        }
    )));
    assert!(
        changes(&e).is_empty(),
        "previous observations stay current after a failed partial"
    );
    // Deadline exceeded on a final: late output discarded, utterance closed.
    let e = run(&mut p, final_(s, 1, 2, "A B", 300));
    assert!(e.iter().any(|e| matches!(
        e,
        SemanticEvent::Degraded {
            reason: DegradeReason::DeadlineExceeded,
            ..
        }
    )));
    assert!(
        changes(&e)
            .iter()
            .all(|c| c.0 == ObservationChange::Retracted)
    );
    assert!(e.iter().any(|e| matches!(
        e,
        SemanticEvent::UtteranceFinalized {
            observations: 0,
            ..
        }
    )));
    // Two consecutive errors → fatal.
    let mut out = Vec::new();
    p.process(
        input(s, 2, 0, RevisionStatus::Partial, "", "C", 400),
        0.0,
        &mut out,
    )
    .unwrap();
    let err = p
        .process(
            input(s, 2, 1, RevisionStatus::Partial, "", "C D", 500),
            0.0,
            &mut out,
        )
        .unwrap_err();
    assert_eq!(err.code(), "semantic.scripted_error");
    let c = p.counters();
    assert_eq!((c.adapter_errors, c.deadline_exceeded), (3, 1));
}

#[test]
fn locality_policy_and_unsupported_features_are_enforced() {
    let script = Script::new(vec![]);
    let remote = ScriptedFactory::new(script.clone()).with_locality(Locality::Remote);
    let cfg = SemanticConfig::default();
    let err = SemanticProcessor::new(
        ProcessorSetup {
            stream: stream(1),
            config: cfg.clone(),
            capabilities: remote.capabilities(),
        },
        remote.create(&cfg).unwrap(),
    )
    .unwrap_err();
    assert_eq!(err.code(), "semantic.locality_not_permitted");
    let mut allowed = cfg.clone();
    allowed.policy.allow_remote = true;
    SemanticProcessor::new(
        ProcessorSetup {
            stream: stream(1),
            config: allowed,
            capabilities: remote.capabilities(),
        },
        remote.create(&cfg).unwrap(),
    )
    .unwrap();
    let local = ScriptedFactory::new(script.clone()).with_locality(Locality::LocalService);
    assert!(
        SemanticProcessor::new(
            ProcessorSetup {
                stream: stream(1),
                config: cfg.clone(),
                capabilities: local.capabilities()
            },
            local.create(&cfg).unwrap(),
        )
        .is_err()
    );

    let mut f = ScriptedFactory::new(script);
    f.capabilities.acts = false;
    let err = SemanticProcessor::new(
        ProcessorSetup {
            stream: stream(1),
            config: cfg.clone(),
            capabilities: f.capabilities(),
        },
        f.create(&cfg).unwrap(),
    )
    .unwrap_err();
    assert_eq!(err.code(), "semantic.feature_unsupported");
    let mut cfg2 = cfg;
    cfg2.acts = false;
    SemanticProcessor::new(
        ProcessorSetup {
            stream: stream(1),
            config: cfg2.clone(),
            capabilities: f.capabilities(),
        },
        f.create(&cfg2).unwrap(),
    )
    .unwrap();
}

#[test]
fn only_the_host_confirms_and_confirmation_survives_unchanged_revisions() {
    let s = stream(1);
    let mut p = rules(no_acts(), s);
    let e = run(
        &mut p,
        input(s, 1, 0, RevisionStatus::Partial, "TEN DOLLARS", "", 100),
    );
    let id = observations(&e)[0].id.clone();
    let mut out = Vec::new();
    p.confirm(&id, &mut out).unwrap();
    let o = &observations(&out)[0];
    assert_eq!(o.epistemic, Epistemic::UserConfirmedFact);
    assert!(matches!(
        out[0],
        SemanticEvent::Observation {
            change: ObservationChange::Confirmed,
            ..
        }
    ));
    let e = run(&mut p, final_(s, 1, 1, "TEN DOLLARS", 200));
    let o = &observations(&e)[0];
    assert_eq!(
        o.epistemic,
        Epistemic::UserConfirmedFact,
        "same content keeps the confirmation"
    );
    assert_eq!(o.status, ObservationStatus::Final);
    assert!(p.confirm(&ObservationId("nope".into()), &mut out).is_err());
}

#[test]
fn replay_is_deterministic() {
    let s = stream(1);
    let script = |p: &mut SemanticProcessor| {
        let mut all = Vec::new();
        for i in [
            input(s, 1, 0, RevisionStatus::Partial, "", "BOOK IT", 300),
            input(s, 1, 1, RevisionStatus::Partial, "BOOK IT", "AT NINE", 900),
            final_(s, 1, 2, "BOOK IT AT NINE THIRTY", 1_500),
            final_(s, 2, 0, "YES THEN WORKS", 2_500),
        ] {
            let mut out = Vec::new();
            p.process(i, 0.0, &mut out).unwrap();
            all.extend(out);
        }
        serde_json::to_string(&all).unwrap()
    };
    let a = script(&mut rules(SemanticConfig::default(), s));
    let b = script(&mut rules(SemanticConfig::default(), s));
    assert_eq!(a, b);
    assert!(
        a.contains("\"resolution\":{\"outcome\":\"resolved\""),
        "THEN resolves to the time: {a}"
    );
}

#[test]
fn domain_profiles_add_terms_without_manufacturing_certainty() {
    let profile: DomainProfile = serde_json::from_str(
        r#"{"schema_version":1,"id":"test.home","version":"1.0.0","language":"en",
            "categories":["device","room"],
            "terms":[{"canonical":"thermostat","category":"device","aliases":["thermo stat"]},
                     {"canonical":"kitchen","category":"room"},
                     {"canonical":"kitchen light","category":"device"},
                     {"canonical":"hall","category":"room"},
                     {"canonical":"hall","category":"device"}],
            "hints":[{"category":"device","referents":["it"]},{"category":"room","referents":["there"]}]}"#,
    )
    .unwrap();
    let s = stream(1);
    let cfg = SemanticConfig {
        profile: Some(profile.clone()),
        acts: false,
        ..SemanticConfig::default()
    };
    let mut p = rules(cfg, s);
    let e = run(
        &mut p,
        final_(s, 1, 0, "SET THE THERMO STAT IN THE KITCHEN", 1_000),
    );
    let obs = observations(&e);
    let ent: Vec<_> = obs
        .iter()
        .filter_map(|o| match &o.payload {
            ObservationPayload::Entity(e) => Some((
                e.category.clone(),
                e.mention.clone(),
                e.normalized.clone(),
                o.uncertainty.level,
                o.provenance.profile.clone(),
            )),
            _ => None,
        })
        .collect();
    assert_eq!(ent[0].0, "profile:device");
    assert_eq!(
        ent[0].2,
        Some(NormalizedValue::Canonical("thermostat".into()))
    );
    assert_eq!(ent[0].3, Certainty::Medium, "an alias is at most Medium");
    assert_eq!(ent[1].1, "KITCHEN");
    assert_eq!(ent[1].3, Certainty::High);
    assert_eq!(
        ent[0].4.as_ref().unwrap().fingerprint,
        profile.fingerprint()
    );
    // Hints widen compatibility: "it" → device, "there" → room.
    let e = run(
        &mut p,
        final_(s, 2, 0, "TURN IT UP AND KEEP IT THERE", 2_000),
    );
    let refs: Vec<_> = observations(&e)
        .into_iter()
        .filter_map(|o| match o.payload {
            ObservationPayload::Reference(r) => Some((r.mention, r.resolution, r.candidates.len())),
            _ => None,
        })
        .collect();
    assert!(
        matches!(&refs[0].1, Resolution::Resolved { .. }),
        "{refs:?}"
    );
    assert!(
        matches!(&refs[2].1, Resolution::Resolved { .. }),
        "{refs:?}"
    );
    // An alias shared by two terms is an ambiguous Low hypothesis without
    // a normalized value.
    let e = run(&mut p, final_(s, 3, 0, "THE HALL", 3_000));
    let o = observations(&e).into_iter().next().unwrap();
    assert_eq!(o.uncertainty.level, Certainty::Low);
    assert_eq!(o.epistemic, Epistemic::Hypothesis);
    let ObservationPayload::Entity(en) = o.payload else {
        panic!()
    };
    assert_eq!(en.category, "profile:ambiguous");
    assert_eq!(en.normalized, None);
}
