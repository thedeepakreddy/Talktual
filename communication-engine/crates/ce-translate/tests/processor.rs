//! Translation processor: revision lifecycle, language policy, selection
//! and fallback, privacy, context, terminology, validation and failures.

use std::time::Duration;

use ce_core::ids::AudioStreamId;
use ce_core::semantic::Locality;
use ce_core::speech::{
    AbstainReason, LanguageDecision, LanguageObservation, LanguageScope, LanguageSource,
    LanguageTag, RevisionStatus, TranscriptRevision,
};
use ce_core::translation::{
    PartialPolicy, SourceLanguagePolicy, TermKind, TerminologyProfile, TranslationConfig,
    TranslationEvent, TranslationRejectReason, TranslationState, UnavailableReason,
    UncertaintyFlag, WithdrawReason,
};
use ce_translate::eval::revision_for;
use ce_translate::{
    AdapterOutcome, AdapterSlot, Faults, MemoryFactory, ProcessorSetup, ScriptedFactory,
    TranslationFactory, TranslationMemory, TranslationProcessor, check_setup,
    scripted_capabilities,
};

fn tag(s: &str) -> LanguageTag {
    LanguageTag::parse(s).unwrap()
}

fn stream() -> AudioStreamId {
    AudioStreamId::from_uuid(uuid::Uuid::from_u128(42))
}

fn cfg(partials: PartialPolicy) -> TranslationConfig {
    TranslationConfig {
        target: Some(tag("es")),
        source: SourceLanguagePolicy::Explicit {
            language: tag("en"),
        },
        partials,
        ..TranslationConfig::default()
    }
}

fn slot(f: &dyn TranslationFactory, c: &TranslationConfig) -> AdapterSlot {
    AdapterSlot {
        capabilities: f.capabilities(),
        adapter: f.create(c).unwrap(),
    }
}

fn processor(f: &dyn TranslationFactory, c: TranslationConfig) -> TranslationProcessor {
    let mut p = TranslationProcessor::new(ProcessorSetup {
        stream: stream(),
        adapters: vec![slot(f, &c)],
        config: c,
        terminology: None,
    })
    .unwrap();
    p.load().unwrap();
    p
}

fn echo() -> ScriptedFactory {
    ScriptedFactory::echo(scripted_capabilities("t.echo", &[("en", "es")]))
}

fn rev(
    utt: u64,
    n: u32,
    status: RevisionStatus,
    committed: &str,
    provisional: &str,
) -> TranscriptRevision {
    let mut r = revision_for(stream(), utt, committed);
    r.revision = n;
    r.status = status;
    r.text.committed = committed.into();
    r.text.provisional = provisional.into();
    r
}

fn run(p: &mut TranslationProcessor, revs: &[TranscriptRevision]) -> Vec<TranslationEvent> {
    let mut out = Vec::new();
    for r in revs {
        p.process(r, 1.0, &mut out).unwrap();
    }
    out
}

fn results(evs: &[TranslationEvent]) -> Vec<(u32, TranslationState, u32, String)> {
    evs.iter()
        .filter_map(|e| match e {
            TranslationEvent::Result(r) => {
                Some((r.sequence, r.state, r.source.revision, r.text.clone()))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn finals_only_by_default_with_exact_source_linkage() {
    let mut p = processor(&echo(), cfg(PartialPolicy::Off));
    let evs = run(
        &mut p,
        &[
            rev(0, 0, RevisionStatus::Partial, "", "hello"),
            rev(0, 1, RevisionStatus::Partial, "hello", "there"),
            rev(0, 2, RevisionStatus::Final, "hello there", ""),
        ],
    );
    assert_eq!(
        results(&evs),
        vec![(0, TranslationState::Final, 2, "[es] HELLO THERE".into())]
    );
    let TranslationEvent::Result(r) = evs
        .iter()
        .find(|e| matches!(e, TranslationEvent::Result(_)))
        .unwrap()
    else {
        unreachable!()
    };
    assert_eq!(
        (r.source.utterance, r.source.revision_status, r.source.chars),
        (0, RevisionStatus::Final, 11)
    );
    assert_eq!(
        r.source.frame_range,
        (0, 8_000),
        "frame range copied from the transcript"
    );
    assert_eq!(r.source_language.language, tag("en"));
    assert_eq!(r.provenance.adapter, "t.echo");
    assert_eq!(
        r.provenance.config_fingerprint,
        cfg(PartialPolicy::Off).fingerprint()
    );
    assert!(r.flags.is_empty());
}

#[test]
fn committed_and_full_partial_policies_supersede_in_order() {
    let mut p = processor(&echo(), cfg(PartialPolicy::Committed));
    let evs = run(
        &mut p,
        &[
            rev(0, 0, RevisionStatus::Partial, "", "the"),
            rev(0, 1, RevisionStatus::Partial, "the", "bus"),
            rev(0, 2, RevisionStatus::Partial, "the", "bus leaves"),
            rev(0, 3, RevisionStatus::Partial, "the bus", "leaves"),
            rev(0, 4, RevisionStatus::Final, "the bus leaves", ""),
        ],
    );
    assert_eq!(
        results(&evs),
        vec![
            (0, TranslationState::Stable, 1, "[es] THE".into()),
            (1, TranslationState::Stable, 3, "[es] THE BUS".into()),
            (2, TranslationState::Final, 4, "[es] THE BUS LEAVES".into()),
        ],
        "stable results only when committed text grows; gapless sequences"
    );
    let TranslationEvent::Result(r0) = &evs
        .iter()
        .find(|e| matches!(e, TranslationEvent::Result(_)))
        .unwrap()
    else {
        unreachable!()
    };
    assert_eq!(r0.flags, vec![UncertaintyFlag::ProvisionalSource]);
    let mut p = processor(&echo(), cfg(PartialPolicy::Full));
    let evs = run(
        &mut p,
        &[
            rev(0, 0, RevisionStatus::Partial, "", "the"),
            rev(0, 1, RevisionStatus::Partial, "the", "bus"),
            rev(0, 2, RevisionStatus::Final, "the bus", ""),
        ],
    );
    let states: Vec<_> = results(&evs).into_iter().map(|r| r.1).collect();
    assert_eq!(
        states,
        vec![
            TranslationState::Provisional,
            TranslationState::Provisional,
            TranslationState::Final
        ]
    );
}

#[test]
fn stale_duplicate_and_late_inputs_are_rejected_never_translated() {
    let f = echo();
    let mut p = processor(&f, cfg(PartialPolicy::Full));
    let evs = run(
        &mut p,
        &[
            rev(0, 3, RevisionStatus::Partial, "", "newer"),
            rev(0, 1, RevisionStatus::Partial, "", "older"),
            rev(0, 3, RevisionStatus::Partial, "", "newer"),
            rev(0, 5, RevisionStatus::Final, "final", ""),
            rev(0, 6, RevisionStatus::Partial, "", "late"),
        ],
    );
    let rejects: Vec<_> = evs
        .iter()
        .filter_map(|e| {
            if let TranslationEvent::InputRejected { reason, .. } = e {
                Some(*reason)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        rejects,
        vec![
            TranslationRejectReason::Stale,
            TranslationRejectReason::Duplicate,
            TranslationRejectReason::AfterFinal
        ]
    );
    assert_eq!(results(&evs).len(), 2);
    assert_eq!(f.calls(), 2, "rejected inputs never reach the adapter");
    assert!(
        !results(&evs)
            .iter()
            .any(|r| r.3.contains("OLDER") || r.3.contains("LATE"))
    );
}

#[test]
fn cancellation_reset_stop_and_failed_finals_withdraw_current_results() {
    let mut p = processor(&echo(), cfg(PartialPolicy::Full));
    let evs = run(
        &mut p,
        &[
            rev(0, 0, RevisionStatus::Partial, "", "a"),
            rev(0, 1, RevisionStatus::Cancelled, "", ""),
        ],
    );
    assert!(evs.iter().any(|e| matches!(
        e,
        TranslationEvent::Withdrawn {
            utterance: 0,
            through_sequence: Some(0),
            reason: WithdrawReason::SourceCancelled,
            ..
        }
    )));
    let evs = run(&mut p, &[rev(1, 0, RevisionStatus::Partial, "", "b")]);
    assert_eq!(results(&evs).len(), 1);
    let mut out = Vec::new();
    p.reset(&mut out);
    assert!(matches!(
        out[..],
        [TranslationEvent::Withdrawn {
            utterance: 1,
            reason: WithdrawReason::Reset,
            ..
        }]
    ));
    let evs = run(
        &mut p,
        &[
            rev(2, 0, RevisionStatus::Partial, "", "c"),
            rev(3, 0, RevisionStatus::Final, "d", ""),
        ],
    );
    assert_eq!(results(&evs).len(), 2);
    out.clear();
    p.finish(
        ce_core::translation::TranslationEndReason::Stopped,
        &mut out,
    );
    let withdrawn: Vec<_> = out
        .iter()
        .filter_map(|e| {
            if let TranslationEvent::Withdrawn {
                utterance, reason, ..
            } = e
            {
                Some((*utterance, *reason))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        withdrawn,
        vec![(2, WithdrawReason::Stopped)],
        "finished utterances keep their final"
    );
    assert!(matches!(out.last(), Some(TranslationEvent::Stopped { .. })));
    // A final that cannot be translated withdraws the utterance's earlier results.
    let f = ScriptedFactory::new(scripted_capabilities("t", &[("en", "es")]), |r| {
        if r.partial {
            AdapterOutcome::Translated {
                text: "x".into(),
                score: None,
                detected_source: None,
            }
        } else {
            AdapterOutcome::NoCoverage
        }
    });
    let mut p = processor(&f, cfg(PartialPolicy::Full));
    let evs = run(
        &mut p,
        &[
            rev(0, 0, RevisionStatus::Partial, "", "a"),
            rev(0, 1, RevisionStatus::Final, "a b", ""),
        ],
    );
    assert!(evs.iter().any(|e| matches!(
        e,
        TranslationEvent::Unavailable {
            reason: UnavailableReason::NoCoverage,
            ..
        }
    )));
    assert!(evs.iter().any(|e| matches!(
        e,
        TranslationEvent::Withdrawn {
            reason: WithdrawReason::FinalUnavailable,
            through_sequence: Some(0),
            ..
        }
    )));
}

fn observed(lang: Option<&str>) -> Option<LanguageObservation> {
    Some(LanguageObservation {
        scope: LanguageScope::Utterance { utterance: 0 },
        decision: match lang {
            Some(l) => LanguageDecision::Identified { language: tag(l) },
            None => LanguageDecision::Abstained {
                reason: AbstainReason::InsufficientEvidence,
            },
        },
        candidates: vec![],
        score_semantics: None,
        source: LanguageSource::TextIdentifier {
            id: "ce.lid.whichlang".into(),
            version: "1".into(),
        },
        evidence: 20,
        fingerprint: "f".into(),
    })
}

#[test]
fn source_language_policy_is_explicit_and_never_silently_guessed() {
    let detected = TranslationConfig {
        source: SourceLanguagePolicy::Detected,
        ..cfg(PartialPolicy::Off)
    };
    // Phase 4 identified English: used, with its provenance.
    let mut p = processor(&echo(), detected.clone());
    let mut r = rev(0, 0, RevisionStatus::Final, "hello", "");
    r.language = observed(Some("en"));
    let evs = run(&mut p, &[r.clone()]);
    let TranslationEvent::Result(res) = &evs[0] else {
        panic!("{evs:?}")
    };
    assert!(matches!(
        res.source_language.provenance,
        ce_core::translation::SourceLanguageProvenance::Recognition { .. }
    ));
    // Phase 4 abstained and the adapter cannot detect: unavailable, not guessed.
    let mut r2 = rev(1, 0, RevisionStatus::Final, "hello", "");
    r2.language = observed(None);
    let evs = run(&mut p, &[r2.clone()]);
    assert!(matches!(
        evs[0],
        TranslationEvent::Unavailable {
            reason: UnavailableReason::UncertainSource,
            ..
        }
    ));
    // An adapter that detects may decide, and says so.
    let mut caps = scripted_capabilities("t.detect", &[("en", "es"), ("fr", "es")]);
    caps.detects_source = true;
    let f = ScriptedFactory::new(caps, |_| AdapterOutcome::Translated {
        text: "hola".into(),
        score: None,
        detected_source: Some(LanguageTag::parse("fr").unwrap()),
    });
    let mut p = processor(&f, detected);
    let evs = run(&mut p, &[r2]);
    let TranslationEvent::Result(res) = &evs[0] else {
        panic!("{evs:?}")
    };
    assert_eq!(res.source_language.language, tag("fr"));
    assert!(res.flags.contains(&UncertaintyFlag::DetectedSourceLanguage));
    // Explicit wins over a disagreeing detection, which is flagged.
    let mut p = processor(&echo(), cfg(PartialPolicy::Off));
    let mut r3 = r;
    r3.language = observed(Some("fr"));
    let evs = run(&mut p, &[r3]);
    let TranslationEvent::Result(res) = &evs[0] else {
        panic!()
    };
    assert_eq!(res.source_language.language, tag("en"));
    assert!(
        res.flags
            .contains(&UncertaintyFlag::SourceLanguageDisagreement)
    );
    // Same language: nothing to translate.
    let same = TranslationConfig {
        target: Some(tag("es")),
        source: SourceLanguagePolicy::Detected,
        ..TranslationConfig::default()
    };
    let mut p = processor(&echo(), same);
    let mut r4 = rev(0, 0, RevisionStatus::Final, "hola", "");
    r4.language = observed(Some("es"));
    assert!(matches!(
        run(&mut p, &[r4])[0],
        TranslationEvent::Unavailable {
            reason: UnavailableReason::SameLanguage,
            ..
        }
    ));
}

#[test]
fn selection_fallback_unsupported_pairs_and_privacy_are_explicit() {
    // Unsupported pair, no fallback: typed outcome.
    let fr = TranslationConfig {
        target: Some(tag("fr")),
        ..cfg(PartialPolicy::Off)
    };
    let mut p = processor(&echo(), fr.clone());
    let evs = run(&mut p, &[rev(0, 0, RevisionStatus::Final, "hi", "")]);
    assert!(
        matches!(&evs[0], TranslationEvent::Unavailable { reason: UnavailableReason::UnsupportedPair, adapters, .. } if adapters == &vec!["t.echo".to_string()])
    );
    // Explicit fallback: memory has no coverage → scripted fallback, recorded.
    let tm = TranslationMemory::new(&[]).unwrap();
    let mem = MemoryFactory::new(tm);
    let fb = echo();
    let c = TranslationConfig {
        fallback: vec!["t.echo".into()],
        ..cfg(PartialPolicy::Off)
    };
    let mut p = TranslationProcessor::new(ProcessorSetup {
        stream: stream(),
        adapters: vec![slot(&mem, &c), slot(&fb, &c)],
        config: c,
        terminology: None,
    })
    .unwrap();
    let evs = run(&mut p, &[rev(0, 0, RevisionStatus::Final, "hi", "")]);
    let TranslationEvent::Result(r) = &evs[0] else {
        panic!("{evs:?}")
    };
    assert_eq!(
        (
            r.provenance.adapter.as_str(),
            r.provenance.fallback_from.as_deref()
        ),
        ("t.echo", Some("ce.translate.memory"))
    );
    // A remote adapter is refused without explicit opt-in (never a silent fallback).
    let mut caps = scripted_capabilities("t.remote", &[("en", "es")]);
    caps.locality = Locality::Remote;
    let remote = ScriptedFactory::echo(caps);
    let c = cfg(PartialPolicy::Off);
    let setup = ProcessorSetup {
        stream: stream(),
        adapters: vec![slot(&echo(), &c), slot(&remote, &c)],
        config: c.clone(),
        terminology: None,
    };
    assert_eq!(
        check_setup(&setup).unwrap_err().code(),
        "translation.remote_not_allowed"
    );
    let allowed = TranslationConfig {
        privacy: ce_core::translation::TranslationPrivacy { allow_remote: true },
        ..c
    };
    let setup = ProcessorSetup {
        stream: stream(),
        adapters: vec![slot(&remote, &allowed)],
        config: allowed,
        terminology: None,
    };
    assert!(check_setup(&setup).is_ok());
    // Partials with an adapter that cannot take partial input: refused at start.
    let setup = ProcessorSetup {
        stream: stream(),
        adapters: vec![slot(&mem, &fr)],
        config: cfg(PartialPolicy::Full),
        terminology: None,
    };
    assert_eq!(
        check_setup(&setup).unwrap_err().code(),
        "translation.partials_unsupported"
    );
    let no_target = ProcessorSetup {
        stream: stream(),
        adapters: vec![slot(&echo(), &fr)],
        config: TranslationConfig::default(),
        terminology: None,
    };
    assert_eq!(
        check_setup(&no_target).unwrap_err().code(),
        "translation.no_target"
    );
}

fn hotel() -> TerminologyProfile {
    serde_json::from_str(include_str!(
        "../../../fixtures/translation/terminology-hospitality-en-es.json"
    ))
    .unwrap()
}

#[test]
fn protection_terminology_and_fidelity_flags() {
    // Placeholder-preserving adapter: digits and do-not-translate terms survive.
    let c = TranslationConfig {
        terminology: Some("hospitality-en-es".into()),
        ..cfg(PartialPolicy::Off)
    };
    let f = ScriptedFactory::new(scripted_capabilities("t", &[("en", "es")]), |r| {
        assert!(
            !r.text.contains("Talktual") && !r.text.contains("412"),
            "protected spans are masked: {}",
            r.text
        );
        assert!(
            !r.terms.is_empty(),
            "terminology passed to an adapter that declares it"
        );
        AdapterOutcome::Translated {
            text: r
                .text
                .replace("room", "habitación")
                .replace("check-in desk", "mostrador de registro"),
            score: Some(0.5),
            detected_source: None,
        }
    });
    let mut p = TranslationProcessor::new(ProcessorSetup {
        stream: stream(),
        adapters: vec![slot(&f, &c)],
        config: c.clone(),
        terminology: Some(hotel()),
    })
    .unwrap();
    let evs = run(
        &mut p,
        &[rev(
            0,
            0,
            RevisionStatus::Final,
            "Talktual room 412 near the check-in desk",
            "",
        )],
    );
    let TranslationEvent::Result(r) = &evs[0] else {
        panic!("{evs:?}")
    };
    assert_eq!(
        r.text,
        "Talktual habitación 412 near the mostrador de registro"
    );
    assert!(r.flags.is_empty(), "{:?}", r.flags);
    assert_eq!(
        (r.provenance.protected_spans, r.provenance.terms_applied),
        (2, 2)
    );
    assert_eq!(
        r.provenance.terminology.as_ref().unwrap().id,
        "hospitality-en-es"
    );
    // An adapter that drops a number or a term is flagged, never corrected.
    let bad = ScriptedFactory::new(scripted_capabilities("t", &[("en", "es")]), |_| {
        AdapterOutcome::Translated {
            text: "la recepción ⟦1⟧".into(),
            score: None,
            detected_source: None,
        }
    });
    let mut p = TranslationProcessor::new(ProcessorSetup {
        stream: stream(),
        adapters: vec![slot(&bad, &c)],
        config: c.clone(),
        terminology: Some(hotel()),
    })
    .unwrap();
    let evs = run(
        &mut p,
        &[rev(
            0,
            0,
            RevisionStatus::Final,
            "Talktual check-in at 7",
            "",
        )],
    );
    let TranslationEvent::Result(r) = &evs[0] else {
        panic!("{evs:?}")
    };
    assert!(r.flags.contains(&UncertaintyFlag::ProtectedSpanLost));
    assert!(r.flags.contains(&UncertaintyFlag::TermMissing));
    assert_eq!(
        r.text, "la recepción 7",
        "restored what was present, invented nothing"
    );
    // Over-application guard: "roommate" is not "room service"; "check-in"
    // inside "check-ins" is not matched.
    let p2 = ce_translate::fidelity::find_terms("my roommate's check-ins", &hotel().terms);
    assert!(p2.is_empty(), "{p2:?}");
    // Profile for another pair: refused.
    let mut wrong = hotel();
    wrong.pair.target = tag("fr");
    let setup = ProcessorSetup {
        stream: stream(),
        adapters: vec![slot(&echo(), &c)],
        config: c,
        terminology: Some(wrong),
    };
    assert_eq!(
        check_setup(&setup).unwrap_err().code(),
        "translation.terminology_mismatch"
    );
    assert_eq!(
        hotel()
            .terms
            .iter()
            .filter(|t| t.kind == TermKind::DoNotTranslate)
            .count(),
        1
    );
}

#[test]
fn context_hints_are_bounded_optional_and_never_override_the_source() {
    use ce_core::semantic::{
        Certainty, Entity, Epistemic, Evidence, EvidenceKind, ObservationId, ObservationPayload,
        ObservationStatus, Provenance, SemanticObservation, SourceRef, TextSpan, Uncertainty,
    };
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let s2 = seen.clone();
    let f = ScriptedFactory::new(scripted_capabilities("t", &[("en", "es")]), move |r| {
        s2.lock().unwrap().push(
            r.hints
                .iter()
                .map(|h| (h.mention.clone(), h.same_utterance))
                .collect::<Vec<_>>(),
        );
        AdapterOutcome::Translated {
            text: "ok".into(),
            score: None,
            detected_source: None,
        }
    });
    let mut p = processor(&f, cfg(PartialPolicy::Off));
    let obs = |utt: u64, mention: &str, status: ObservationStatus| SemanticObservation {
        id: ObservationId(format!("o{utt}{mention}")),
        schema_version: 1,
        version: 1,
        payload: ObservationPayload::Entity(Entity {
            category: "time".into(),
            mention: mention.into(),
            normalized: None,
            attributes: vec![],
        }),
        status,
        epistemic: Epistemic::Observation,
        uncertainty: Uncertainty {
            level: Certainty::High,
            score: None,
            calibrated: false,
        },
        evidence: vec![Evidence {
            kind: EvidenceKind::Explicit,
            span: None,
            context_item: None,
            rule: "x".into(),
        }],
        source: SourceRef {
            stream: stream(),
            utterance: utt,
            revision: 0,
            revision_status: RevisionStatus::Final,
            span: TextSpan { start: 0, end: 1 },
            committed: true,
        },
        provenance: Provenance {
            adapter: "r".into(),
            adapter_version: "1".into(),
            config_fingerprint: "f".into(),
            profile: None,
            locality: Locality::InProcess,
        },
        produced_at: Default::default(),
    };
    p.observe(&obs(0, "seven", ObservationStatus::Final));
    p.observe(&obs(1, "eight", ObservationStatus::Final));
    p.observe(&obs(1, "nine", ObservationStatus::Provisional)); // not final: ignored
    p.observe(&obs(1, "ten", ObservationStatus::Final)); // not in the source text
    let evs = run(
        &mut p,
        &[rev(1, 0, RevisionStatus::Final, "at eight please", "")],
    );
    let TranslationEvent::Result(r) = &evs[0] else {
        panic!("{evs:?}")
    };
    assert!(
        r.flags.contains(&UncertaintyFlag::ContextConflict),
        "a hint that disagrees with the source is dropped and flagged"
    );
    assert_eq!(
        seen.lock().unwrap()[0],
        vec![("eight".to_string(), true), ("seven".to_string(), false)]
    );
    assert_eq!(r.provenance.context_hints, 2);
    // Disabled context: no hints at all.
    let off = TranslationConfig {
        context: ce_core::translation::ContextPolicy {
            use_semantic: false,
            ..Default::default()
        },
        ..cfg(PartialPolicy::Off)
    };
    let seen2 = std::sync::Arc::new(std::sync::Mutex::new(0usize));
    let s3 = seen2.clone();
    let f = ScriptedFactory::new(scripted_capabilities("t", &[("en", "es")]), move |r| {
        *s3.lock().unwrap() += r.hints.len();
        AdapterOutcome::Translated {
            text: "ok".into(),
            score: None,
            detected_source: None,
        }
    });
    let mut p = processor(&f, off);
    p.observe(&obs(0, "seven", ObservationStatus::Final));
    run(&mut p, &[rev(0, 0, RevisionStatus::Final, "seven", "")]);
    assert_eq!(*seen2.lock().unwrap(), 0);
}

#[test]
fn malformed_output_bounds_timeouts_and_errors_are_typed() {
    let cases: Vec<(&str, UnavailableReason)> = vec![
        ("", UnavailableReason::MalformedOutput),
        ("bad\u{7}", UnavailableReason::MalformedOutput),
        ("x ⟦9⟧", UnavailableReason::MalformedOutput),
    ];
    for (text, reason) in cases {
        let t = text.to_string();
        let f = ScriptedFactory::new(scripted_capabilities("t", &[("en", "es")]), move |_| {
            AdapterOutcome::Translated {
                text: t.clone(),
                score: Some(f32::NAN),
                detected_source: None,
            }
        });
        let mut p = processor(&f, cfg(PartialPolicy::Off));
        let evs = run(&mut p, &[rev(0, 0, RevisionStatus::Final, "hi", "")]);
        assert!(
            matches!(&evs[0], TranslationEvent::Unavailable { reason: r, .. } if *r == reason),
            "{text:?}: {evs:?}"
        );
    }
    // Output over the bound.
    let mut c = cfg(PartialPolicy::Off);
    c.budget.max_output_chars = 5;
    let mut p = processor(&echo(), c);
    assert!(matches!(
        run(
            &mut p,
            &[rev(0, 0, RevisionStatus::Final, "hello there", "")]
        )[0],
        TranslationEvent::Unavailable {
            reason: UnavailableReason::OutputTooLong,
            ..
        }
    ));
    // Input over the bound and empty input.
    let mut c = cfg(PartialPolicy::Off);
    c.budget.max_input_chars = 3;
    let mut p = processor(&echo(), c);
    assert!(matches!(
        run(&mut p, &[rev(0, 0, RevisionStatus::Final, "hello", "")])[0],
        TranslationEvent::Unavailable {
            reason: UnavailableReason::InputTooLong,
            ..
        }
    ));
    assert!(matches!(
        run(&mut p, &[rev(1, 0, RevisionStatus::Final, "  ", "")])[0],
        TranslationEvent::Unavailable {
            reason: UnavailableReason::EmptySource,
            ..
        }
    ));
    // A late result is discarded, not presented.
    let mut c = cfg(PartialPolicy::Off);
    c.budget.deadline_ms = 20;
    let slow = echo().with_faults(Faults {
        delay: Duration::from_millis(60),
        ..Faults::default()
    });
    let mut p = processor(&slow, c);
    let evs = run(&mut p, &[rev(0, 0, RevisionStatus::Final, "hi", "")]);
    assert!(evs.iter().any(|e| matches!(
        e,
        TranslationEvent::Unavailable {
            reason: UnavailableReason::Timeout,
            ..
        }
    )));
    assert!(!evs.iter().any(|e| matches!(e, TranslationEvent::Result(_))));
    // Errors: reported per input; repeated errors fail the processor; no
    // automatic retry unless configured (and then bounded).
    let errs = echo().with_faults(Faults {
        error_at: vec![0, 1, 2],
        ..Faults::default()
    });
    let mut c = cfg(PartialPolicy::Off);
    c.budget.max_consecutive_errors = 3;
    let mut p = processor(&errs, c.clone());
    let mut out = Vec::new();
    p.process(&rev(0, 0, RevisionStatus::Final, "a", ""), 0.0, &mut out)
        .unwrap();
    p.process(&rev(1, 0, RevisionStatus::Final, "b", ""), 0.0, &mut out)
        .unwrap();
    assert!(
        p.process(&rev(2, 0, RevisionStatus::Final, "c", ""), 0.0, &mut out)
            .is_err()
    );
    assert_eq!(errs.calls(), 3, "no automatic retry by default");
    let flaky = echo().with_faults(Faults {
        error_at: vec![0],
        ..Faults::default()
    });
    c.budget.max_retries = 1;
    let mut p = processor(&flaky, c);
    let evs = run(&mut p, &[rev(0, 0, RevisionStatus::Final, "a", "")]);
    assert!(matches!(evs[0], TranslationEvent::Result(_)));
    assert_eq!(flaky.calls(), 2, "one bounded retry");
}

#[test]
fn translation_memory_never_invents_and_is_deterministic() {
    let tm = ce_translate::eval::load_memory().unwrap();
    let expected = tm
        .lookup(&tag("en"), &tag("es"), "Where is the bathroom, please?")
        .unwrap()
        .to_string();
    let f = MemoryFactory::new(tm);
    let caps = f.capabilities();
    assert!(
        !caps.partial_input
            && !caps.detects_source
            && caps.deterministic
            && caps.score_semantics.is_none()
    );
    let mut p = processor(&f, cfg(PartialPolicy::Off));
    let evs = run(
        &mut p,
        &[
            rev(
                0,
                0,
                RevisionStatus::Final,
                "where is the bathroom please",
                "",
            ),
            rev(
                1,
                0,
                RevisionStatus::Final,
                "where is the bathroom tonight",
                "",
            ),
        ],
    );
    let TranslationEvent::Result(r) = &evs[0] else {
        panic!("{evs:?}")
    };
    assert_eq!(
        r.text, expected,
        "normalized exact match (case and punctuation ignored)"
    );
    assert!(matches!(
        evs[1],
        TranslationEvent::Unavailable {
            reason: UnavailableReason::NoCoverage,
            ..
        }
    ));
}

#[test]
fn transcripts_and_observations_are_not_mutated() {
    let mut p = processor(&echo(), cfg(PartialPolicy::Full));
    let r = rev(0, 0, RevisionStatus::Partial, "hello", "world");
    let before = r.clone();
    let mut out = Vec::new();
    p.process(&r, 0.0, &mut out).unwrap();
    assert_eq!(r, before, "the transcript revision is read-only");
}

// ------------------------------------------------------------------ audit regressions

fn entity_obs(
    utt: u64,
    mention: &str,
    status: ce_core::semantic::ObservationStatus,
    epistemic: ce_core::semantic::Epistemic,
) -> ce_core::semantic::SemanticObservation {
    use ce_core::semantic::{
        Certainty, Entity, Evidence, EvidenceKind, ObservationId, ObservationPayload, Provenance,
        SemanticObservation, SourceRef, TextSpan, Uncertainty,
    };
    SemanticObservation {
        id: ObservationId(format!("o{utt}-{mention}")),
        schema_version: 1,
        version: 1,
        payload: ObservationPayload::Entity(Entity {
            category: "time".into(),
            mention: mention.into(),
            normalized: None,
            attributes: vec![],
        }),
        status,
        epistemic,
        uncertainty: Uncertainty {
            level: Certainty::High,
            score: None,
            calibrated: false,
        },
        evidence: vec![Evidence {
            kind: EvidenceKind::Explicit,
            span: None,
            context_item: None,
            rule: "x".into(),
        }],
        source: SourceRef {
            stream: stream(),
            utterance: utt,
            revision: 0,
            revision_status: RevisionStatus::Final,
            span: TextSpan { start: 0, end: 1 },
            committed: true,
        },
        provenance: Provenance {
            adapter: "r".into(),
            adapter_version: "1".into(),
            config_fingerprint: "f".into(),
            profile: None,
            locality: Locality::InProcess,
        },
        produced_at: Default::default(),
    }
}

/// D1: a final whose translation fails the processor must not leave the
/// utterance's earlier results current.
#[test]
fn audit_d1_failed_final_withdraws_earlier_results() {
    let f = echo().with_faults(Faults {
        error_at: vec![1],
        ..Faults::default()
    });
    let mut c = cfg(PartialPolicy::Full);
    c.budget.max_consecutive_errors = 1;
    let mut p = processor(&f, c);
    let mut out = Vec::new();
    p.process(&rev(0, 0, RevisionStatus::Partial, "", "a"), 0.0, &mut out)
        .unwrap();
    assert!(matches!(out[0], TranslationEvent::Result(_)));
    let err = p.process(&rev(0, 1, RevisionStatus::Final, "a b", ""), 0.0, &mut out);
    assert!(err.is_err(), "repeated errors fail the processor");
    p.fail(&err.unwrap_err(), &mut out);
    assert!(
        out.iter().any(|e| matches!(
            e,
            TranslationEvent::Withdrawn {
                utterance: 0,
                through_sequence: Some(0),
                ..
            }
        )),
        "the provisional result of utterance 0 must be withdrawn: {:?}",
        out.iter().map(TranslationEvent::kind).collect::<Vec<_>>()
    );
}

/// D2: after a reset, results never restart at a sequence a consumer has
/// already seen (or withdrawn).
#[test]
fn audit_d2_reset_keeps_sequences_monotonic() {
    let mut p = processor(&echo(), cfg(PartialPolicy::Full));
    let mut out = Vec::new();
    for n in 0..3 {
        p.process(
            &rev(
                0,
                n,
                RevisionStatus::Partial,
                "",
                &"w ".repeat(n as usize + 1),
            ),
            0.0,
            &mut out,
        )
        .unwrap();
    }
    p.reset(&mut out);
    let withdrawn = out.iter().find_map(|e| match e {
        TranslationEvent::Withdrawn {
            through_sequence, ..
        } => *through_sequence,
        _ => None,
    });
    assert_eq!(withdrawn, Some(2));
    out.clear();
    p.process(
        &rev(0, 3, RevisionStatus::Final, "w w w w", ""),
        0.0,
        &mut out,
    )
    .unwrap();
    for e in &out {
        if let TranslationEvent::Result(r) = e {
            assert!(
                r.sequence > 2,
                "sequence {} reuses a withdrawn number",
                r.sequence
            );
        }
    }
}

/// D3/D4: only stated (not inferred) final entities become hints, and a
/// retracted one is removed.
#[test]
fn audit_d3_d4_hints_are_stated_and_follow_retraction() {
    use ce_core::semantic::{Epistemic, ObservationStatus};
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let s2 = seen.clone();
    let f = ScriptedFactory::new(scripted_capabilities("t", &[("en", "es")]), move |r| {
        s2.lock().unwrap().push(
            r.hints
                .iter()
                .map(|h| h.mention.clone())
                .collect::<Vec<_>>(),
        );
        AdapterOutcome::Translated {
            text: "ok".into(),
            score: None,
            detected_source: None,
        }
    });
    let mut p = processor(&f, cfg(PartialPolicy::Off));
    p.observe(&entity_obs(
        0,
        "seven",
        ObservationStatus::Final,
        Epistemic::Observation,
    ));
    p.observe(&entity_obs(
        0,
        "maybe",
        ObservationStatus::Final,
        Epistemic::Hypothesis,
    ));
    p.observe(&entity_obs(
        0,
        "linked",
        ObservationStatus::Final,
        Epistemic::ResolvedInterpretation,
    ));
    p.observe(&entity_obs(
        0,
        "eight",
        ObservationStatus::Final,
        Epistemic::Observation,
    ));
    p.observe(&entity_obs(
        0,
        "eight",
        ObservationStatus::Retracted,
        Epistemic::Observation,
    ));
    run(&mut p, &[rev(1, 0, RevisionStatus::Final, "x", "")]);
    assert_eq!(seen.lock().unwrap()[0], vec!["seven".to_string()]);
}

/// D5: terms are reported as applied only by an adapter that applies them
/// (or when they were protected).
#[test]
fn audit_d5_terms_applied_reflects_the_adapter() {
    let mem = MemoryFactory::new(
        TranslationMemory::new(&[ce_translate::memory::Segment {
            source_language: tag("en"),
            target_language: tag("es"),
            source: "where is check-in".into(),
            target: "dónde está el registro".into(),
            provenance: "t".into(),
        }])
        .unwrap(),
    );
    let c = TranslationConfig {
        terminology: Some("hospitality-en-es".into()),
        ..cfg(PartialPolicy::Off)
    };
    let mut p = TranslationProcessor::new(ProcessorSetup {
        stream: stream(),
        adapters: vec![slot(&mem, &c)],
        config: c,
        terminology: Some(hotel()),
    })
    .unwrap();
    let evs = run(
        &mut p,
        &[rev(0, 0, RevisionStatus::Final, "where is check-in", "")],
    );
    let TranslationEvent::Result(r) = &evs[0] else {
        panic!("{evs:?}")
    };
    assert_eq!(
        r.provenance.terms_applied, 0,
        "the memory adapter applies no terminology"
    );
    assert!(
        r.flags.is_empty(),
        "the target term is present: verified, not applied"
    );
}
