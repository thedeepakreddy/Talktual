//! Pinned evaluation gate: replay is deterministic; the dev-only memory is
//! exact on dev and has no coverage on the held-out test split (no
//! leakage); the oracle scores 100 (metric sanity); identity scores are
//! pinned (metric regression). Values are those in
//! `docs/phases/phase-6-evaluation.md`; changing a metric or the corpus
//! means re-recording both.

use std::collections::BTreeMap;

use ce_translate::eval::{self, Item};
use ce_translate::{AdapterOutcome, MemoryFactory, ScriptedFactory, scripted_capabilities};

fn by_pair<'a>(items: &'a [Item], split: &str) -> BTreeMap<String, Vec<&'a Item>> {
    let mut m: BTreeMap<String, Vec<&Item>> = BTreeMap::new();
    for it in items.iter().filter(|i| i.split == split) {
        m.entry(format!("{}-{}", it.source_language, it.target_language))
            .or_default()
            .push(it);
    }
    m
}

#[test]
fn replay_is_deterministic_and_the_memory_does_not_leak() {
    let corpus = eval::load_corpus().unwrap();
    let memory = MemoryFactory::new(eval::load_memory().unwrap());
    for split in ["dev", "test"] {
        for (pair, items) in by_pair(&corpus, split) {
            let a = eval::replay(&memory, &items, None).unwrap();
            let b = eval::replay(&memory, &items, None).unwrap();
            let strip = |v: &[eval::Produced]| {
                v.iter()
                    .map(|p| (p.id.clone(), p.output.clone()))
                    .collect::<Vec<_>>()
            };
            assert_eq!(strip(&a), strip(&b), "{pair} {split}");
            let r = eval::score(&pair, &items, &a);
            if split == "dev" {
                assert_eq!(
                    (r.covered, r.exact),
                    (r.items, r.items),
                    "{pair}: memory exact on dev"
                );
            } else {
                assert_eq!(
                    r.covered, 0,
                    "{pair}: no held-out item may be in the memory"
                );
                assert_eq!(
                    r.unavailable.get("NoCoverage"),
                    Some(&r.items),
                    "{pair}: honest no-coverage, nothing invented"
                );
            }
        }
    }
}

#[test]
fn oracle_and_identity_bounds_are_pinned() {
    let corpus = eval::load_corpus().unwrap();
    let pinned: BTreeMap<&str, (f64, f64)> = [
        ("en-de", (0.6891, 16.8285)),
        ("en-es", (2.3812, 18.0898)),
        ("en-fr", (0.7824, 16.4882)),
        ("es-en", (0.7912, 16.9666)),
    ]
    .into_iter()
    .collect();
    let refs: BTreeMap<(String, String), String> = corpus
        .iter()
        .map(|i| {
            (
                (i.source.clone(), i.target_language.clone()),
                i.reference.clone(),
            )
        })
        .collect();
    let refs = std::sync::Arc::new(refs);
    for (pair, items) in by_pair(&corpus, "test") {
        let (s, t) = pair.split_once('-').unwrap();
        let identity = ScriptedFactory::new(scripted_capabilities("identity", &[(s, t)]), |r| {
            AdapterOutcome::Translated {
                text: r.text.to_string(),
                score: None,
                detected_source: None,
            }
        });
        let r = eval::score(
            &pair,
            &items,
            &eval::replay(&identity, &items, None).unwrap(),
        );
        let (b, c) = pinned[pair.as_str()];
        assert!(
            (r.bleu_all - b).abs() < 1e-3 && (r.chrf_all - c).abs() < 1e-3,
            "{pair}: identity {} / {}",
            r.bleu_all,
            r.chrf_all
        );
        let rr = refs.clone();
        let mut caps = scripted_capabilities("oracle", &[(s, t)]);
        caps.placeholders = false;
        let oracle = ScriptedFactory::new(caps, move |q| {
            match rr.get(&(q.text.to_string(), q.target.primary().to_string())) {
                Some(x) => AdapterOutcome::Translated {
                    text: x.clone(),
                    score: None,
                    detected_source: None,
                },
                None => AdapterOutcome::NoCoverage,
            }
        });
        let r = eval::score(&pair, &items, &eval::replay(&oracle, &items, None).unwrap());
        assert!(
            (r.bleu_all - 100.0).abs() < 1e-9 && (r.chrf_all - 100.0).abs() < 1e-9,
            "{pair}: oracle"
        );
        assert_eq!(
            (r.number_errors, r.negation_errors, r.question_errors),
            (0, 0, 0),
            "{pair}: screens agree with references"
        );
    }
}
