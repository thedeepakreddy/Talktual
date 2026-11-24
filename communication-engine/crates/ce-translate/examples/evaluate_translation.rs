//! Evaluate translation adapters on the authored corpus, pair by pair.
//!
//! ```text
//! cargo run --release -p ce-translate --example evaluate_translation -- test
//! cargo run --release -p ce-translate --example evaluate_translation -- dev
//! cargo run --release -p ce-translate --example evaluate_translation -- test --dump DIR   # hyps/refs for sacreBLEU
//! ```
//!
//! Adapters: `ce.translate.memory` (memory built from the dev split only),
//! `identity` (copies the source: a lower bound) and `oracle` (returns the
//! reference: a metric sanity check, BLEU/chrF should be 100). Neither
//! identity nor oracle is a translation system.

#![allow(clippy::print_stdout)]

use std::collections::BTreeMap;
use std::sync::Arc;

use ce_translate::eval::{self, Item, PairReport};
use ce_translate::{
    AdapterOutcome, MemoryFactory, ScriptedFactory, TranslationFactory, scripted_capabilities,
};

fn row(adapter: &str, r: &PairReport) {
    let f = |v: Option<f64>| v.map_or("—".to_string(), |x| format!("{x:.1}"));
    let unavailable: Vec<String> = r
        .unavailable
        .iter()
        .map(|(k, v)| format!("{k}:{v}"))
        .collect();
    println!(
        "| {} | {adapter} | {} | {}/{} | {} | {} | {:.1} | {:.1} | {} | {} | {} | {} | {} |",
        r.pair,
        r.items,
        r.covered,
        r.items,
        f(r.bleu_covered),
        f(r.chrf_covered),
        r.bleu_all,
        r.chrf_all,
        r.exact,
        r.number_errors,
        r.negation_errors,
        r.question_errors,
        if unavailable.is_empty() {
            "—".into()
        } else {
            unavailable.join(", ")
        },
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("score") {
        // `score HYP REF`: CE's corpus BLEU and chrF for two line files
        // (used to cross-check against sacreBLEU).
        let read = |p: &str| -> Vec<String> {
            std::fs::read_to_string(p)
                .expect("file")
                .lines()
                .map(str::to_owned)
                .collect()
        };
        let (h, r) = (read(&args[2]), read(&args[3]));
        println!(
            "BLEU {:.4} chrF {:.4}",
            eval::bleu(&h, &r),
            eval::chrf(&h, &r)
        );
        return;
    }
    if args.get(1).map(String::as_str) == Some("overhead") {
        // Engine overhead per final through the processor with a trivial
        // adapter (no translation work): validation, protection, fidelity
        // checks, event construction. Wall clock, this machine only.
        let corpus = eval::load_corpus().expect("corpus");
        let items: Vec<&Item> = corpus
            .iter()
            .filter(|i| i.source_language == "en" && i.target_language == "es")
            .collect();
        let echo = ScriptedFactory::echo(scripted_capabilities("echo", &[("en", "es")]));
        let term = eval::load_terminology().expect("terminology");
        let mut runs = Vec::new();
        for _ in 0..200 {
            let t0 = std::time::Instant::now();
            let _ = eval::replay(&echo, &items, Some(term.clone())).expect("replay");
            runs.push(t0.elapsed().as_secs_f64() * 1e6 / items.len() as f64);
        }
        runs.sort_by(f64::total_cmp);
        println!(
            "per-request engine overhead over {} requests x 200 runs: p50 {:.1} us, p95 {:.1} us, max {:.1} us",
            items.len(),
            runs[100],
            runs[190],
            runs[199]
        );
        return;
    }
    let split = args.get(1).cloned().unwrap_or_else(|| "test".into());
    let dump = args
        .iter()
        .position(|a| a == "--dump")
        .and_then(|i| args.get(i + 1))
        .cloned();
    let corpus = eval::load_corpus().expect("corpus");
    let memory = MemoryFactory::new(eval::load_memory().expect("memory"));
    let terminology = eval::load_terminology().expect("terminology");
    let refs: Arc<BTreeMap<(String, String), String>> = Arc::new(
        corpus
            .iter()
            .map(|i| {
                (
                    (i.source.clone(), i.target_language.clone()),
                    i.reference.clone(),
                )
            })
            .collect(),
    );
    let mut by_pair: BTreeMap<String, Vec<&Item>> = BTreeMap::new();
    for it in corpus.iter().filter(|i| i.split == split) {
        by_pair
            .entry(format!("{}-{}", it.source_language, it.target_language))
            .or_default()
            .push(it);
    }
    println!("\n### {split} split\n");
    println!(
        "| pair | adapter | items | covered | BLEU (covered) | chrF (covered) | BLEU (all) | chrF (all) | exact | number err | negation err | question err | unavailable |"
    );
    println!("|---|---|---|---|---|---|---|---|---|---|---|---|---|");
    let mut categories: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for (pair, items) in &by_pair {
        let (s, t) = pair.split_once('-').unwrap();
        let pairs = [(s, t)];
        let identity = ScriptedFactory::new(scripted_capabilities("identity", &pairs), |r| {
            AdapterOutcome::Translated {
                text: r.text.to_string(),
                score: None,
                detected_source: None,
            }
        });
        let refs2 = refs.clone();
        let mut oc = scripted_capabilities("oracle", &pairs);
        oc.placeholders = false;
        let oracle = ScriptedFactory::new(oc, move |r| {
            match refs2.get(&(r.text.to_string(), r.target.primary().to_string())) {
                Some(x) => AdapterOutcome::Translated {
                    text: x.clone(),
                    score: None,
                    detected_source: None,
                },
                None => AdapterOutcome::NoCoverage,
            }
        });
        let adapters: Vec<(&str, &dyn TranslationFactory)> = vec![
            ("ce.translate.memory", &memory),
            ("identity", &identity),
            ("oracle", &oracle),
        ];
        for (name, f) in adapters {
            let term = (pair == "en-es").then(|| terminology.clone());
            let produced = eval::replay(f, items, term).expect("replay");
            let report = eval::score(pair, items, &produced);
            row(name, &report);
            if name == "ce.translate.memory" {
                for (it, p) in items.iter().zip(&produced) {
                    let e = categories
                        .entry(format!("{pair}/{}", it.category))
                        .or_default();
                    e.0 += 1;
                    e.1 += usize::from(p.output.is_ok());
                }
            }
            if let Some(dir) = &dump {
                std::fs::create_dir_all(dir).expect("dump dir");
                let hyps: Vec<String> = produced
                    .iter()
                    .map(|p| p.output.clone().unwrap_or_default())
                    .collect();
                let refs: Vec<String> = items.iter().map(|i| i.reference.clone()).collect();
                std::fs::write(format!("{dir}/{pair}.{name}.hyp"), hyps.join("\n") + "\n").unwrap();
                std::fs::write(format!("{dir}/{pair}.ref"), refs.join("\n") + "\n").unwrap();
            }
        }
    }
    println!("\n### translation-memory coverage by pair/category ({split})\n");
    println!("| pair/category | items | covered |");
    println!("|---|---|---|");
    for (k, (n, c)) in categories {
        println!("| {k} | {n} | {c} |");
    }
}
