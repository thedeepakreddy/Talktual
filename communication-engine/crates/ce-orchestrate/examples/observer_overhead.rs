//! Observer overhead per event (Phase 7 evaluation). Synthetic workload:
//! `UTTERANCES` utterances × (4 partial + 1 final) transcript revisions,
//! each followed by one translation result and, for finals, one semantic
//! finalization — fed straight to the observer (no runtime, no model).
//! Repeats the run `RUNS` times and prints per-event mean, p50/p95 of the
//! per-run means, and the bounded-state size. Release build recommended:
//!
//! `cargo run --release -p ce-orchestrate --example observer_overhead`

#![allow(clippy::print_stdout)]

use std::time::Instant;

use ce_core::ids::AudioStreamId;
use ce_core::orchestration::{DeadlinePolicy, TimingUncertainty};
use ce_core::semantic::SemanticEvent;
use ce_core::speech::RevisionStatus;
use ce_core::time::MonotonicTime;
use ce_core::translation::{
    TranslationEvent, TranslationProvenance, TranslationResult, TranslationSource,
    TranslationState, TranslationTiming,
};
use ce_orchestrate::Observer;
use ce_translate::eval::revision_for;

const UTTERANCES: u64 = 20_000;
const RUNS: usize = 5;

fn main() {
    let stream = AudioStreamId::from_uuid(uuid::Uuid::from_u128(1));
    let mut per_event_ns = Vec::new();
    let mut remembered = 0;
    let mut events = 0u64;
    for _ in 0..RUNS {
        let mut o = Observer::new(
            16_000,
            256,
            DeadlinePolicy::default(),
            TimingUncertainty::Unknown,
        );
        let mut n = 0u64;
        let t0 = Instant::now();
        for u in 0..UTTERANCES {
            for rev in 0..5u32 {
                let mut r = revision_for(stream, u, "a few words here");
                r.revision = rev;
                r.status = if rev == 4 {
                    RevisionStatus::Final
                } else {
                    RevisionStatus::Partial
                };
                let base = 1_000_000_000 + u * 1_000_000_000;
                r.meta.time_range = (
                    MonotonicTime::from_nanos(base),
                    MonotonicTime::from_nanos(base + 500_000_000),
                );
                let now = MonotonicTime::from_nanos(base + 800_000_000);
                o.on_transcript(&r, now);
                let result = TranslationEvent::Result(Box::new(TranslationResult {
                    sequence: rev,
                    state: if rev == 4 {
                        TranslationState::Final
                    } else {
                        TranslationState::Provisional
                    },
                    source: TranslationSource {
                        stream,
                        utterance: u,
                        revision: rev,
                        revision_status: r.status,
                        chars: 16,
                        frame_range: r.meta.frame_range,
                        time_range: r.meta.time_range,
                    },
                    source_language: ce_core::translation::SourceLanguageRef {
                        language: ce_core::speech::LanguageTag::parse("en").unwrap(),
                        provenance: ce_core::translation::SourceLanguageProvenance::Explicit,
                    },
                    target: ce_core::speech::LanguageTag::parse("es").unwrap(),
                    text: String::new(),
                    flags: vec![],
                    score: None,
                    provenance: TranslationProvenance {
                        adapter: "synthetic".into(),
                        version: "1".into(),
                        model_fingerprint: None,
                        config_fingerprint: "f".into(),
                        locality: ce_core::semantic::Locality::InProcess,
                        terminology: None,
                        terms_applied: 0,
                        context_hints: 0,
                        protected_spans: 0,
                        fallback_from: None,
                    },
                    timing: TranslationTiming {
                        queued_ms: 1.0,
                        adapter_ms: 2.0,
                    },
                }));
                o.on_translation(&result, now);
                n += 2;
                if rev == 4 {
                    o.on_semantic(
                        &SemanticEvent::UtteranceFinalized {
                            stream,
                            utterance: u,
                            revision: rev,
                            observations: 0,
                        },
                        now,
                    );
                    n += 1;
                }
            }
        }
        let el = t0.elapsed();
        per_event_ns.push(el.as_nanos() as f64 / n as f64);
        remembered = o.remembered();
        events = n;
        assert_eq!(o.counters().causal_violations, 0);
    }
    let mean = per_event_ns.iter().sum::<f64>() / per_event_ns.len() as f64;
    let mut sorted = per_event_ns.clone();
    sorted.sort_by(f64::total_cmp);
    let var = per_event_ns.iter().map(|x| (x - mean).powi(2)).sum::<f64>()
        / (per_event_ns.len() as f64 - 1.0);
    println!("observer overhead: {RUNS} runs × {events} events");
    println!(
        "per event: mean {:.0} ns, min {:.0} ns, max {:.0} ns, sd {:.0} ns",
        mean,
        sorted[0],
        sorted[sorted.len() - 1],
        var.sqrt()
    );
    println!(
        "runs (ns/event): {:?}",
        per_event_ns.iter().map(|x| x.round()).collect::<Vec<_>>()
    );
    println!("utterances remembered at the end: {remembered} (bound 256)");
}
