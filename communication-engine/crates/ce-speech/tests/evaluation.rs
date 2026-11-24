//! Regression gate for the Phase 4 evaluation (held-out test split).
//!
//! Thresholds sit **below** the values reported in
//! `docs/phases/phase-4-evaluation.md`, so they catch regressions without
//! being tuned to the test data. Evidence: synthetic replay; transcripts
//! from the oracle reference adapter (pipeline behaviour, not ASR quality).

use std::sync::Arc;

use ce_audio_intel::eval::{Condition, Format};
use ce_core::speech::LanguageIdConfig;
use ce_speech::WhichlangIdentifier;
use ce_speech::eval::{self, ReplayOptions, Split};
use ce_speech::fake::{ReferenceFactory, ReferenceNoise};
use ce_speech::language::LanguageIdentifier;

fn lid() -> Option<Arc<dyn LanguageIdentifier>> {
    Some(Arc::new(WhichlangIdentifier))
}

#[test]
fn oracle_pipeline_on_the_held_out_split() {
    let utts = eval::load_split(Split::Test).unwrap();
    let sc = eval::build("gate", &utts, Condition::Clean, Format::BASE);
    let f = ReferenceFactory::new(sc.references.clone(), ReferenceNoise::default());
    let r = eval::replay(&sc, &f, lid(), &ReplayOptions::default()).unwrap();
    let m = eval::score(&sc, &r);
    // Reported: 6/6 finals, 0 missed/split/merged, WER 0.000.
    assert_eq!(m.finals, 6);
    assert_eq!(
        m.missed_references + m.split_references + m.merged_finals,
        0
    );
    assert!(m.wer.rate() <= 0.02, "oracle WER {}", m.wer.rate());
    // Reported p95 first partial 400 ms, final 790 ms after speech end.
    assert!(m.first_partial_ms.p95 <= 700.0, "{:?}", m.first_partial_ms);
    assert!(m.final_ms.p95 <= 1_200.0, "{:?}", m.final_ms);
    assert_eq!(m.commit_amendments, 0);
    // Deterministic replay.
    let r2 = eval::replay(&sc, &f, lid(), &ReplayOptions::default()).unwrap();
    assert_eq!(eval::digest(&r.events), eval::digest(&r2.events));

    // A different device format gives the same transcripts.
    let st = eval::build(
        "gate",
        &utts,
        Condition::Clean,
        Format {
            rate: 48_000,
            channels: 2,
            block_frames: 480,
        },
    );
    let fs = ReferenceFactory::new(st.references.clone(), ReferenceNoise::default());
    let ms = eval::score(
        &st,
        &eval::replay(&st, &fs, lid(), &ReplayOptions::default()).unwrap(),
    );
    assert_eq!(ms.finals, 6);
    assert!(ms.wer.rate() <= 0.02);

    // Recognizer-only drops split utterances but lose little (reported
    // WER 0.015).
    let o = ReplayOptions {
        drops: vec![(200, 10), (800, 10), (1_400, 10)],
        ..Default::default()
    };
    let md = eval::score(&sc, &eval::replay(&sc, &f, lid(), &o).unwrap());
    assert!(md.wer.rate() <= 0.06, "drops WER {}", md.wer.rate());
    assert!(md.split_references >= 1);
}

#[test]
fn language_identification_on_the_held_out_split() {
    let items = eval::load_lid_corpus().unwrap();
    let r = eval::evaluate_lid(&items, &WhichlangIdentifier, &LanguageIdConfig::default());
    let all = r.groups[&("test".to_owned(), "all".to_owned())];
    // Reported: accuracy 0.984 on decided, coverage 0.73.
    assert!(all.accuracy() >= 0.95, "accuracy {}", all.accuracy());
    assert!(all.coverage() >= 0.65, "coverage {}", all.coverage());
    let long = r.groups[&("test".to_owned(), "long".to_owned())];
    assert!(long.accuracy() >= 0.95 && long.coverage() >= 0.9);
    // Reported: 7 of 8 mixed-language items abstain as ambiguous.
    let ambiguous = r
        .mixed
        .iter()
        .filter(|(_, o)| o == "abstain:Ambiguous")
        .count();
    assert!(ambiguous >= 6, "{:?}", r.mixed);
}
