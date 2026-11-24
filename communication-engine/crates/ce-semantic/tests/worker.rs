//! Worker gates: non-blocking push, coalescing, overload, supervision
//! (panic, stall), stop/drain, reset, confirmation and isolation between
//! concurrent workers.

use std::time::{Duration, Instant};

use ce_core::ids::AudioStreamId;
use ce_core::semantic::*;
use ce_core::speech::RevisionStatus;
use ce_core::time::MonotonicTime;
use ce_semantic::scripted::{Script, ScriptedFactory, Step};
use ce_semantic::{
    ProcessorSetup, PushOutcome, RulesFactory, SemanticFactory, SemanticHandle, SemanticProcessor,
    SemanticSignal, spawn_semantic,
};
use tokio::sync::mpsc;

fn stream(n: u128) -> AudioStreamId {
    AudioStreamId::from_uuid(uuid::Uuid::from_u128(n))
}

fn input(
    s: AudioStreamId,
    utt: u64,
    rev: u32,
    status: RevisionStatus,
    text: &str,
) -> SemanticInput {
    SemanticInput {
        stream: s,
        utterance: utt,
        revision: rev,
        status,
        committed_chars: if status == RevisionStatus::Partial {
            0
        } else {
            text.chars().count() as u32
        },
        text: text.into(),
        language: None,
        recognition_score: None,
        time_range: (
            MonotonicTime::from_nanos(0),
            MonotonicTime::from_nanos(u64::from(rev) * 1_000_000 + 1),
        ),
        audio_ms: 100,
    }
}

fn spawn(
    factory: &dyn SemanticFactory,
    config: SemanticConfig,
    s: AudioStreamId,
) -> (SemanticHandle, mpsc::Receiver<SemanticSignal>) {
    let p = SemanticProcessor::new(
        ProcessorSetup {
            stream: s,
            config: config.clone(),
            capabilities: factory.capabilities(),
        },
        factory.create(&config).unwrap(),
    )
    .unwrap();
    let (tx, rx) = mpsc::channel(4_096);
    (spawn_semantic(p, tx).unwrap(), rx)
}

fn collect(
    rx: &mut mpsc::Receiver<SemanticSignal>,
    timeout: Duration,
) -> (Vec<SemanticEvent>, Option<SemanticStatus>) {
    let deadline = Instant::now() + timeout;
    let mut events = Vec::new();
    while Instant::now() < deadline {
        match rx.try_recv() {
            Ok(SemanticSignal::Event(e)) => events.push(e),
            Ok(SemanticSignal::Ended { status, .. }) => return (events, Some(*status)),
            Err(mpsc::error::TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(2)),
            Err(mpsc::error::TryRecvError::Disconnected) => break,
        }
    }
    (events, None)
}

#[test]
fn partials_are_coalesced_and_finals_are_never_dropped_while_there_is_room() {
    let script = Script::new(vec![Step::Sleep(Duration::from_millis(80))]);
    let f = ScriptedFactory::new(script.clone());
    let cfg = SemanticConfig {
        acts: false,
        budget: SemanticBudget {
            queue: 4,
            ..SemanticBudget::default()
        },
        ..SemanticConfig::default()
    };
    let s = stream(1);
    let (h, mut rx) = spawn(&f, cfg, s);
    // The first input occupies the adapter for 80 ms.
    assert_eq!(
        h.push(input(s, 1, 0, RevisionStatus::Partial, "A")),
        PushOutcome::Queued
    );
    std::thread::sleep(Duration::from_millis(20));
    let t0 = Instant::now();
    let mut outcomes = Vec::new();
    for r in 1..=10 {
        outcomes.push(h.push(input(s, 1, r, RevisionStatus::Partial, "A B")));
    }
    assert!(
        t0.elapsed() < Duration::from_millis(20),
        "push never blocks"
    );
    assert_eq!(outcomes[0], PushOutcome::Queued);
    assert!(outcomes[1..].iter().all(|o| *o == PushOutcome::Coalesced));
    assert_eq!(h.push(final_(s, 1, 11, "A B C")), PushOutcome::Queued);
    h.stop(SemanticEndReason::SourceEnded);
    let (events, status) = collect(&mut rx, Duration::from_secs(5));
    let status = status.expect("ended");
    let calls: Vec<u32> = script.lock().unwrap().calls.iter().map(|c| c.1).collect();
    assert_eq!(
        calls,
        vec![0, 10, 11],
        "only the newest queued partial is interpreted; the final is drained"
    );
    assert_eq!(status.counters.coalesced, 9);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SemanticEvent::UtteranceFinalized { revision: 11, .. }))
    );
    assert_eq!(status.ended, Some(SemanticEndReason::SourceEnded));
}

fn final_(s: AudioStreamId, utt: u64, rev: u32, text: &str) -> SemanticInput {
    input(s, utt, rev, RevisionStatus::Final, text)
}

#[test]
fn overload_drops_oldest_partials_reports_it_and_keeps_finals() {
    let script = Script::new(vec![Step::Sleep(Duration::from_millis(100))]);
    let f = ScriptedFactory::new(script.clone());
    let cfg = SemanticConfig {
        acts: false,
        budget: SemanticBudget {
            queue: 2,
            ..SemanticBudget::default()
        },
        ..SemanticConfig::default()
    };
    let s = stream(1);
    let (h, mut rx) = spawn(&f, cfg, s);
    h.push(input(s, 1, 0, RevisionStatus::Partial, "A"));
    std::thread::sleep(Duration::from_millis(20));
    // Distinct utterances, so no coalescing: the queue (2) overflows.
    assert_eq!(
        h.push(input(s, 2, 0, RevisionStatus::Partial, "B")),
        PushOutcome::Queued
    );
    assert_eq!(
        h.push(input(s, 3, 0, RevisionStatus::Partial, "C")),
        PushOutcome::Queued
    );
    assert_eq!(
        h.push(input(s, 4, 0, RevisionStatus::Partial, "D")),
        PushOutcome::QueuedDroppingOlder
    );
    assert_eq!(
        h.push(final_(s, 5, 0, "E")),
        PushOutcome::QueuedDroppingOlder
    );
    assert_eq!(
        h.push(final_(s, 6, 0, "F")),
        PushOutcome::QueuedDroppingOlder
    );
    assert_eq!(
        h.push(final_(s, 7, 0, "G")),
        PushOutcome::Queued,
        "finals may use up to twice the capacity"
    );
    assert_eq!(h.push(final_(s, 8, 0, "H")), PushOutcome::Queued);
    assert_eq!(
        h.push(final_(s, 9, 0, "I")),
        PushOutcome::Dropped,
        "hard bound reached"
    );
    h.stop(SemanticEndReason::SourceEnded);
    let (events, status) = collect(&mut rx, Duration::from_secs(5));
    assert!(events.iter().any(|e| matches!(
        e,
        SemanticEvent::Degraded {
            reason: DegradeReason::Overload,
            ..
        }
    )));
    let finals: Vec<u64> = events
        .iter()
        .filter_map(|e| match e {
            SemanticEvent::UtteranceFinalized { utterance, .. } => Some(*utterance),
            _ => None,
        })
        .collect();
    assert_eq!(finals, vec![5, 6, 7, 8]);
    assert!(status.unwrap().counters.coalesced >= 3);
}

#[test]
fn a_panicking_adapter_is_contained_and_reported() {
    let script = Script::new(vec![Step::Panic]);
    let f = ScriptedFactory::new(script);
    let s = stream(1);
    let (h, mut rx) = spawn(&f, SemanticConfig::default(), s);
    h.push(final_(s, 1, 0, "A"));
    let (events, status) = collect(&mut rx, Duration::from_secs(5));
    assert!(events.iter().any(|e| matches!(e, SemanticEvent::Failed { error, .. } if error.code == "semantic.worker_panicked")));
    assert!(matches!(
        events.last(),
        Some(SemanticEvent::Stopped {
            reason: SemanticEndReason::Failed,
            ..
        })
    ));
    let status = status.unwrap();
    assert!(!status.running);
    assert_eq!(status.ended, Some(SemanticEndReason::Failed));
    assert_eq!(h.push(final_(s, 2, 0, "B")), PushOutcome::Dropped);
}

#[test]
fn a_hung_adapter_is_detected_as_stalled_and_abandoned() {
    let script = Script::new(vec![Step::Sleep(Duration::from_millis(600))]);
    let f = ScriptedFactory::new(script);
    let cfg = SemanticConfig {
        budget: SemanticBudget {
            stall_timeout_ms: 150,
            ..SemanticBudget::default()
        },
        ..SemanticConfig::default()
    };
    let s = stream(1);
    let (mut h, mut rx) = spawn(&f, cfg, s);
    h.push(final_(s, 1, 0, "A"));
    let t0 = Instant::now();
    while !h.stalled() && t0.elapsed() < Duration::from_secs(2) {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(h.stalled());
    h.abandon(SemanticEndReason::Failed);
    std::thread::sleep(Duration::from_millis(700));
    let (events, status) = collect(&mut rx, Duration::from_millis(100));
    assert!(status.is_none(), "an abandoned worker sends nothing more");
    assert!(
        events
            .iter()
            .all(|e| !matches!(e, SemanticEvent::Observation { .. }))
    );
    assert!(h.is_finished());
    assert_eq!(h.join_if_finished(), Some(false));
}

#[test]
fn stop_discards_the_queue_reset_clears_context_confirm_reaches_the_processor() {
    let s = stream(1);
    let (h, mut rx) = spawn(
        &RulesFactory,
        SemanticConfig {
            acts: false,
            ..SemanticConfig::default()
        },
        s,
    );
    h.push(final_(s, 1, 0, "TEN DOLLARS"));
    let (events, _) = collect(&mut rx, Duration::from_millis(300));
    let id = events
        .iter()
        .find_map(|e| match e {
            SemanticEvent::Observation { observation, .. } => Some(observation.id.clone()),
            _ => None,
        })
        .unwrap();
    h.confirm(id.clone());
    let (events, _) = collect(&mut rx, Duration::from_millis(200));
    assert!(events.iter().any(|e| matches!(e, SemanticEvent::Observation { change: ObservationChange::Confirmed, observation } if observation.id == id && observation.epistemic == Epistemic::UserConfirmedFact)));
    assert_eq!(h.status().context.items, 1);
    h.request_reset();
    let (events, _) = collect(&mut rx, Duration::from_millis(200));
    assert!(events.iter().any(|e| matches!(
        e,
        SemanticEvent::Context {
            change: ContextChange::Cleared,
            ..
        }
    )));
    assert_eq!(h.status().context.items, 0);
    h.stop(SemanticEndReason::Stopped);
    let (events, status) = collect(&mut rx, Duration::from_secs(2));
    assert!(matches!(
        events.last(),
        Some(SemanticEvent::Stopped {
            reason: SemanticEndReason::Stopped,
            ..
        })
    ));
    assert!(!status.unwrap().running);
}

#[test]
fn one_failing_worker_does_not_affect_another() {
    let bad = ScriptedFactory::new(Script::new(vec![Step::Panic]));
    let (a, b) = (stream(1), stream(2));
    let (ha, mut rxa) = spawn(&bad, SemanticConfig::default(), a);
    let (hb, mut rxb) = spawn(
        &RulesFactory,
        SemanticConfig {
            acts: false,
            ..SemanticConfig::default()
        },
        b,
    );
    ha.push(final_(a, 1, 0, "X"));
    for u in 0..50 {
        hb.push(final_(b, u, 0, "TWO DOLLARS"));
    }
    let (ea, sa) = collect(&mut rxa, Duration::from_secs(2));
    assert!(ea.iter().any(|e| matches!(e, SemanticEvent::Failed { .. })));
    assert_eq!(sa.unwrap().ended, Some(SemanticEndReason::Failed));
    hb.stop(SemanticEndReason::SourceEnded);
    let (eb, sb) = collect(&mut rxb, Duration::from_secs(5));
    let finals = eb
        .iter()
        .filter(|e| matches!(e, SemanticEvent::UtteranceFinalized { .. }))
        .count();
    assert_eq!(finals, 50);
    assert!(eb.iter().all(|e| e.stream() == b), "events are per stream");
    assert_eq!(sb.unwrap().ended, Some(SemanticEndReason::SourceEnded));
}
