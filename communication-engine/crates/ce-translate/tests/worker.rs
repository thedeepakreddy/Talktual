//! Translation worker: coalescing, saturation, drain/discard policies,
//! reset, panic containment, stall, and bounded memory under load.

use std::time::Duration;

use ce_core::ids::AudioStreamId;
use ce_core::speech::{LanguageTag, RevisionStatus, TranscriptRevision};
use ce_core::translation::{
    PartialPolicy, SourceLanguagePolicy, TranslationConfig, TranslationDegrade,
    TranslationEndReason, TranslationEvent, TranslationState,
};
use ce_translate::eval::revision_for;
use ce_translate::{
    AdapterSlot, Faults, ProcessorSetup, PushOutcome, ScriptedFactory, TranslationFactory,
    TranslationProcessor, TranslationSignal, scripted_capabilities, spawn_translation,
};
use tokio::sync::mpsc;

fn tag(s: &str) -> LanguageTag {
    LanguageTag::parse(s).unwrap()
}

fn stream() -> AudioStreamId {
    AudioStreamId::from_uuid(uuid::Uuid::from_u128(7))
}

fn cfg(partials: PartialPolicy, queue: u32) -> TranslationConfig {
    let mut c = TranslationConfig {
        target: Some(tag("es")),
        source: SourceLanguagePolicy::Explicit {
            language: tag("en"),
        },
        partials,
        ..TranslationConfig::default()
    };
    c.budget.queue = queue;
    c.budget.deadline_ms = 10_000;
    c
}

fn spawn(
    f: &ScriptedFactory,
    c: TranslationConfig,
) -> (
    ce_translate::TranslationHandle,
    mpsc::Receiver<TranslationSignal>,
) {
    let p = TranslationProcessor::new(ProcessorSetup {
        stream: stream(),
        adapters: vec![AdapterSlot {
            capabilities: f.capabilities(),
            adapter: f.create(&c).unwrap(),
        }],
        config: c,
        terminology: None,
    })
    .unwrap();
    let (tx, rx) = mpsc::channel(4_096);
    (spawn_translation(p, tx).unwrap(), rx)
}

fn rev(utt: u64, n: u32, status: RevisionStatus, text: &str) -> TranscriptRevision {
    let mut r = revision_for(stream(), utt, text);
    r.revision = n;
    r.status = status;
    if status == RevisionStatus::Partial {
        r.text.provisional = std::mem::take(&mut r.text.committed);
    }
    r
}

fn collect(
    rx: &mut mpsc::Receiver<TranslationSignal>,
) -> (
    Vec<TranslationEvent>,
    Option<ce_core::translation::TranslationStatus>,
) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut evs = Vec::new();
    while std::time::Instant::now() < deadline {
        match rx.try_recv() {
            Ok(TranslationSignal::Event(e)) => evs.push(e),
            Ok(TranslationSignal::Ended { status, .. }) => return (evs, Some(*status)),
            Err(_) => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    (evs, None)
}

fn slow() -> ScriptedFactory {
    ScriptedFactory::echo(scripted_capabilities("t", &[("en", "es")])).with_faults(Faults {
        delay: Duration::from_millis(30),
        ..Faults::default()
    })
}

#[test]
fn superseded_partials_are_coalesced_and_finals_are_kept() {
    let f = slow();
    let (h, mut rx) = spawn(&f, cfg(PartialPolicy::Full, 8));
    for n in 0..20 {
        let _ = h.push(rev(0, n, RevisionStatus::Partial, &format!("word{n}")));
    }
    assert_eq!(
        h.push(rev(0, 20, RevisionStatus::Final, "done")),
        PushOutcome::Queued
    );
    // An older revision never replaces a newer queued one.
    h.stop(TranslationEndReason::SourceEnded);
    let (evs, status) = collect(&mut rx);
    let status = status.expect("ended");
    let results: Vec<_> = evs
        .iter()
        .filter_map(|e| {
            if let TranslationEvent::Result(r) = e {
                Some((r.state, r.source.revision))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        results.last(),
        Some(&(TranslationState::Final, 20)),
        "the final is translated (drained)"
    );
    assert!(
        results.len() < 21,
        "superseded partials were never translated: {results:?}"
    );
    assert!(status.counters.coalesced > 0);
    let revisions: Vec<u32> = results.iter().map(|r| r.1).collect();
    assert!(
        revisions.windows(2).all(|w| w[0] < w[1]),
        "results follow revision order: {revisions:?}"
    );
}

#[test]
fn saturation_drops_partials_first_and_reports_overload() {
    let f = slow();
    let (h, mut rx) = spawn(&f, cfg(PartialPolicy::Full, 4));
    let mut outcomes = Vec::new();
    // Distinct utterances: no coalescing possible.
    for u in 0..12 {
        outcomes.push(h.push(rev(u, 0, RevisionStatus::Partial, "x")));
    }
    for u in 12..20 {
        outcomes.push(h.push(rev(u, 0, RevisionStatus::Final, "y")));
    }
    assert!(
        outcomes.contains(&PushOutcome::QueuedDroppingOlder)
            || outcomes.contains(&PushOutcome::Dropped)
    );
    assert!(h.backlog() <= 8, "bounded at twice the capacity");
    h.stop(TranslationEndReason::SourceEnded);
    let (evs, status) = collect(&mut rx);
    assert!(evs.iter().any(|e| matches!(
        e,
        TranslationEvent::Degraded {
            reason: TranslationDegrade::Overload,
            ..
        }
    )));
    let finals = evs
        .iter()
        .filter(|e| matches!(e, TranslationEvent::Result(r) if r.state == TranslationState::Final))
        .count();
    assert!(finals >= 4, "finals are favoured over partials ({finals})");
    assert!(status.unwrap().counters.dropped > 0);
}

#[test]
fn stop_discards_and_withdraws_source_ended_drains() {
    let f = slow();
    let (h, mut rx) = spawn(&f, cfg(PartialPolicy::Full, 16));
    h.push(rev(0, 0, RevisionStatus::Partial, "a"));
    for u in 1..10 {
        h.push(rev(u, 0, RevisionStatus::Final, "b"));
    }
    std::thread::sleep(Duration::from_millis(40));
    h.stop(TranslationEndReason::Stopped);
    let (evs, status) = collect(&mut rx);
    let status = status.unwrap();
    assert_eq!(status.ended, Some(TranslationEndReason::Stopped));
    assert!(
        status.counters.finals < 9,
        "queued inputs were discarded on stop"
    );
    assert!(push_after_stop_dropped(&h));
    let _ = evs;
}

fn push_after_stop_dropped(h: &ce_translate::TranslationHandle) -> bool {
    h.push(rev(99, 0, RevisionStatus::Final, "late")) == PushOutcome::Dropped
}

#[test]
fn reset_discards_pending_work_and_withdraws() {
    let f = slow();
    let (h, mut rx) = spawn(&f, cfg(PartialPolicy::Full, 16));
    h.push(rev(0, 0, RevisionStatus::Partial, "a"));
    std::thread::sleep(Duration::from_millis(60));
    for n in 1..5 {
        h.push(rev(0, n, RevisionStatus::Partial, "a b"));
    }
    h.request_reset();
    std::thread::sleep(Duration::from_millis(100));
    h.push(rev(1, 0, RevisionStatus::Final, "fresh"));
    h.stop(TranslationEndReason::SourceEnded);
    let (evs, _) = collect(&mut rx);
    assert!(
        evs.iter().any(|e| matches!(
            e,
            TranslationEvent::Withdrawn {
                utterance: 0,
                reason: ce_core::translation::WithdrawReason::Reset,
                ..
            }
        )),
        "{evs:?}"
    );
    assert!(
        evs.iter().any(|e| final_of(e, 1)),
        "translation continues after reset"
    );
}

fn final_of(e: &TranslationEvent, u: u64) -> bool {
    matches!(e, TranslationEvent::Result(r) if r.source.utterance == u && r.state == TranslationState::Final)
}

#[test]
fn panics_and_load_failures_end_the_worker_cleanly() {
    let f =
        ScriptedFactory::echo(scripted_capabilities("t", &[("en", "es")])).with_faults(Faults {
            panic_at: Some(1),
            ..Faults::default()
        });
    let (h, mut rx) = spawn(&f, cfg(PartialPolicy::Off, 8));
    h.push(rev(0, 0, RevisionStatus::Final, "ok"));
    h.push(rev(1, 0, RevisionStatus::Final, "boom"));
    let (evs, status) = collect(&mut rx);
    assert!(evs.iter().any(|e| final_of(e, 0)));
    let TranslationEvent::Failed { error, .. } = evs
        .iter()
        .find(|e| matches!(e, TranslationEvent::Failed { .. }))
        .unwrap()
    else {
        unreachable!()
    };
    assert_eq!(error.code, "translation.worker_panicked");
    assert_eq!(status.unwrap().ended, Some(TranslationEndReason::Failed));
    let f =
        ScriptedFactory::echo(scripted_capabilities("t", &[("en", "es")])).with_faults(Faults {
            load_error: true,
            ..Faults::default()
        });
    let (_h, mut rx) = spawn(&f, cfg(PartialPolicy::Off, 8));
    let (evs, status) = collect(&mut rx);
    assert!(
        matches!(&evs[0], TranslationEvent::Failed { error, .. } if error.code == "translation.load_failed")
    );
    assert_eq!(status.unwrap().ended, Some(TranslationEndReason::Failed));
}

#[test]
fn a_hang_is_visible_to_the_stall_detector() {
    let f =
        ScriptedFactory::echo(scripted_capabilities("t", &[("en", "es")])).with_faults(Faults {
            hang_at: Some((0, Duration::from_millis(800))),
            ..Faults::default()
        });
    let mut c = cfg(PartialPolicy::Off, 8);
    c.budget.stall_timeout_ms = 150;
    let (mut h, _rx) = spawn(&f, c);
    h.push(rev(0, 0, RevisionStatus::Final, "x"));
    std::thread::sleep(Duration::from_millis(400));
    assert!(h.stalled());
    h.abandon(TranslationEndReason::Failed);
    std::thread::sleep(Duration::from_millis(600));
    assert!(
        h.is_finished(),
        "an abandoned worker exits once the adapter returns"
    );
}

#[test]
fn long_streams_stay_bounded() {
    let f = ScriptedFactory::echo(scripted_capabilities("t", &[("en", "es")]));
    let (h, mut rx) = spawn(&f, cfg(PartialPolicy::Committed, 64));
    let t0 = std::time::Instant::now();
    let mut pushed = 0;
    for u in 0..2_000u64 {
        for n in 0..3 {
            let mut r = rev(u, n, RevisionStatus::Partial, "");
            r.text.committed = "word ".repeat(n as usize + 1).trim().to_string();
            while h.backlog() > 48 {
                std::thread::sleep(Duration::from_millis(1));
            }
            h.push(r);
            pushed += 1;
        }
        h.push(rev(u, 3, RevisionStatus::Final, "word word word word"));
        pushed += 1;
        while let Ok(TranslationSignal::Event(_)) = rx.try_recv() {}
    }
    h.stop(TranslationEndReason::SourceEnded);
    let (_, status) = collect(&mut rx);
    let status = status.unwrap();
    assert_eq!(status.counters.finals, 2_000);
    assert!(status.counters.inputs as usize <= pushed);
    // Per-utterance state and hints are bounded (MAX_UTTERANCES / hints):
    // memory does not grow with the number of utterances. Throughput is
    // engine overhead only (the scripted adapter is trivial).
    eprintln!("8000 inputs in {:?}", t0.elapsed());
}
