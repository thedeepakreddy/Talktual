//! Replay regression gate on the semantic corpus (`fixtures/semantic/`).
//!
//! The rule adapter is deterministic, so the scores are pinned exactly.
//! Changing a rule changes them. Re-record them only together with
//! `docs/phases/phase-5-evaluation.md`, and never tune on `test` or `asr`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use ce_core::semantic::{DomainProfile, SemanticConfig};
use ce_semantic::RulesFactory;
use ce_semantic::eval::{SplitReport, evaluate, parse_corpus};

fn report(split: &str) -> SplitReport {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/semantic");
    let corpus =
        parse_corpus(&std::fs::read_to_string(root.join("corpus.jsonl")).unwrap()).unwrap();
    let p: DomainProfile =
        serde_json::from_slice(&std::fs::read(root.join("profiles/home.json")).unwrap()).unwrap();
    p.validate().unwrap();
    let profiles = BTreeMap::from([(p.id.clone(), p)]);
    evaluate(
        &RulesFactory,
        &SemanticConfig::default(),
        &profiles,
        &corpus,
        split,
    )
    .unwrap()
}

fn counts(r: &SplitReport) -> (u32, u32, u32, u32, u32, u32) {
    let e = &r.entities_all.exact;
    (
        e.tp,
        e.fp,
        e.fn_,
        r.references.correct,
        r.references.gold,
        r.entities_all.normalized_correct,
    )
}

#[test]
fn held_out_scores_are_pinned_and_replay_is_robust() {
    let test = report("test");
    assert_eq!((test.dialogues, test.utterances), (15, 45));
    assert_eq!(counts(&test), (34, 3, 4, 11, 15, 34), "test split");
    assert!(
        (test.acts.macro_f1 - 0.929).abs() < 0.001,
        "{}",
        test.acts.macro_f1
    );
    let asr = report("asr");
    assert_eq!(counts(&asr), (2, 1, 3, 0, 0, 2), "asr split");
    assert_eq!(asr.entities_all.lost_to_asr, 3);
    let dev = report("dev");
    assert_eq!(counts(&dev), (28, 0, 0, 8, 11, 28), "dev split");
    for r in [&dev, &test, &asr] {
        let b = &r.robustness;
        assert_eq!(
            b.disorder_consistent, b.disorder_dialogues,
            "{}: stale/duplicate partials change finals",
            r.split
        );
        assert_eq!(
            b.finals_only_consistent, b.finals_only_dialogues,
            "{}: partial path changes finals",
            r.split
        );
        assert!(b.disorder_rejected > 0);
    }
}
